// Copyright (c) 2025-2026, NVIDIA CORPORATION & AFFILIATES. All rights reserved.
//
// Licensed under the Apache License, Version 2.0 (the "License");
// you may not use this file except in compliance with the License.
// You may obtain a copy of the License at
//
//     http://www.apache.org/licenses/LICENSE-2.0
//
// Unless required by applicable law or agreed to in writing, software
// distributed under the License is distributed on an "AS IS" BASIS,
// WITHOUT WARRANTIES OR CONDITIONS OF ANY KIND, either express or implied.
// See the License for the specific language governing permissions and
// limitations under the License.
//
// SPDX-License-Identifier: Apache-2.0

//! Collector-owned admission control for BMC HTTP requests.

use std::collections::{HashMap, HashSet, VecDeque};
use std::convert::Infallible;
use std::error::Error as StdError;
use std::fmt;
use std::future::Future;
use std::hash::BuildHasher;
use std::num::{NonZeroU32, NonZeroUsize};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use http::HeaderMap;
/// Credentials whose rejection [`RequestGate::execute_classified`] records.
pub use nv_redfish::bmc_http::BmcCredentials;
use nv_redfish::bmc_http::{
    CacheableError, HttpClient, HttpPushUriUpdateRequest, MultipartUpdateRequest,
    RejectedUriReferenceError, RequestError,
};
use nv_redfish::core::{
    BmcError, BmcErrorClass, BoxTryStream, DecodeError, ModificationResponse, ODataETag, Repair,
    SessionCreateResponse, UploadReader,
};
use nv_redfish_dispatcher::{
    BoundedConcurrency, ClockConfig, Completion, CostUnits, FixedCost, FutureWork, Readiness,
    RoundRobin, Runtime, RuntimeConfig, RuntimeHandle, RuntimeOutput, ScheduledWork, Scheduler,
    TokenBucket, TokenBucketConfig, WithCost,
};
use serde::{Deserialize, Serialize, de::DeserializeOwned};
use tokio::sync::{Notify, oneshot};
use url::Url;

type Work = FutureWork<(), Infallible>;
type Queue = Arc<Mutex<VecDeque<ScheduledWork<Work, ()>>>>;
type WorkflowQueues = RoundRobin<Work, ()>;
type CostedWorkflows = FixedCost<Work, WorkflowQueues>;
type RateLimitedEndpoint = TokenBucket<Work, CostedWorkflows>;
type Endpoint = BoundedConcurrency<Work, RateLimitedEndpoint>;
type Root = RoundRobin<Work, WithCost<()>>;
type Handle = RuntimeHandle<(), Infallible, WithCost<()>>;

/// Dispatcher limits applied to every configured BMC endpoint.
#[derive(Clone, Copy, Debug)]
pub struct DispatcherConfig {
    /// Maximum requests executing across all BMCs.
    pub global_max_in_flight: usize,
    /// Maximum requests executing against one BMC.
    pub bmc_max_in_flight: u32,
    /// Sustained requests admitted per second for one BMC.
    pub bmc_requests_per_second: u64,
    /// Initial and maximum token-bucket capacity for one BMC.
    pub bmc_burst: u64,
    /// Maximum queued requests for each workflow on each BMC.
    pub workflow_queue_capacity: usize,
}

/// Invalid dispatcher configuration or endpoint set.
#[derive(Debug)]
pub struct ConfigError(String);

impl fmt::Display for ConfigError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(&self.0)
    }
}

impl StdError for ConfigError {}

/// A running dispatcher and its per-endpoint admission gates.
pub struct Dispatcher {
    registry: Mutex<EndpointRegistry>,
    endpoint_config: EndpointConfig,
    lifecycle: Arc<DispatcherLifecycle>,
}

struct EndpointRegistry {
    gates: HashMap<String, EndpointGates>,
    child_ids: HashMap<String, u32>,
}

#[derive(Clone, Copy)]
struct EndpointConfig {
    max_in_flight: NonZeroU32,
    requests_per_second: u64,
    burst: u64,
    queue_capacity: usize,
}

struct DispatcherLifecycle {
    handle: Handle,
    notify: Arc<Notify>,
}

impl Drop for DispatcherLifecycle {
    fn drop(&mut self) {
        self.handle.graceful_shutdown();
        self.notify.notify_one();
    }
}

impl Dispatcher {
    /// Start one dispatcher runtime for the supplied endpoint identifiers.
    pub fn start(
        config: DispatcherConfig,
        endpoint_ids: impl IntoIterator<Item = String>,
    ) -> Result<Self, ConfigError> {
        let global_max_in_flight = NonZeroUsize::new(config.global_max_in_flight)
            .ok_or_else(|| ConfigError("global_max_in_flight must be greater than zero".into()))?;
        let bmc_max_in_flight = NonZeroU32::new(config.bmc_max_in_flight)
            .ok_or_else(|| ConfigError("bmc_max_in_flight must be greater than zero".into()))?;
        if config.bmc_requests_per_second == 0 {
            return Err(ConfigError(
                "bmc_requests_per_second must be greater than zero".into(),
            ));
        }
        if config.bmc_burst == 0 {
            return Err(ConfigError("bmc_burst must be greater than zero".into()));
        }
        if config.workflow_queue_capacity == 0 {
            return Err(ConfigError(
                "workflow_queue_capacity must be greater than zero".into(),
            ));
        }
        let endpoint_config = EndpointConfig {
            max_in_flight: bmc_max_in_flight,
            requests_per_second: config.bmc_requests_per_second,
            burst: config.bmc_burst,
            queue_capacity: config.workflow_queue_capacity,
        };

        let notify = Arc::new(Notify::new());
        let mut root = Root::new();
        let mut pending_gates = HashMap::new();
        let mut child_ids = HashMap::new();
        for endpoint_id in endpoint_ids {
            if pending_gates.contains_key(&endpoint_id) {
                return Err(ConfigError(format!(
                    "duplicate dispatcher endpoint identifier {endpoint_id:?}"
                )));
            }

            let (endpoint, inventory_queue, attestation_queue) = build_endpoint(endpoint_config);
            let child_id = root.add_child(endpoint);
            child_ids.insert(endpoint_id.clone(), child_id);
            pending_gates.insert(endpoint_id, (inventory_queue, attestation_queue));
        }

        let mut runtime = Runtime::new(
            RuntimeConfig {
                global_max_in_flight,
                clock: ClockConfig::Wallclock,
            },
            root,
        );
        let handle = runtime.handle();
        let driver_notify = Arc::clone(&notify);
        tokio::spawn(async move {
            loop {
                match runtime.next().await {
                    RuntimeOutput::Work { .. } => {}
                    RuntimeOutput::SleepUntil(when) => {
                        tokio::select! {
                            () = tokio::time::sleep_until(when.into()) => {}
                            () = driver_notify.notified() => {}
                        }
                    }
                    RuntimeOutput::Shutdown => break,
                    RuntimeOutput::Runtime(event) => match event {},
                }
            }
        });
        let lifecycle = Arc::new(DispatcherLifecycle {
            handle,
            notify: Arc::clone(&notify),
        });

        let gates = pending_gates
            .into_iter()
            .map(|(id, (inventory, attestation))| {
                let gates = EndpointGates::new(
                    &id,
                    inventory,
                    attestation,
                    endpoint_config.queue_capacity,
                    &lifecycle,
                );
                (id, gates)
            })
            .collect();

        Ok(Self {
            registry: Mutex::new(EndpointRegistry { gates, child_ids }),
            endpoint_config,
            lifecycle,
        })
    }

    /// Return the inventory admission gate for an endpoint.
    #[must_use]
    pub fn inventory_gate(&self, endpoint_id: &str) -> Option<RequestGate> {
        self.registry
            .lock()
            .ok()?
            .gates
            .get(endpoint_id)
            .map(|gates| gates.inventory.clone())
    }

    /// Return the attestation admission gate for an endpoint.
    #[must_use]
    pub fn attestation_gate(&self, endpoint_id: &str) -> Option<RequestGate> {
        self.registry
            .lock()
            .ok()?
            .gates
            .get(endpoint_id)
            .map(|gates| gates.attestation.clone())
    }

    /// Make the dispatcher endpoint set match the supplied identifiers.
    ///
    /// Call this between collection cycles, when no request gates from the
    /// previous endpoint set are still in use.
    pub fn sync_endpoints(
        &self,
        endpoint_ids: impl IntoIterator<Item = String>,
    ) -> Result<(), ConfigError> {
        let mut desired = HashSet::new();
        for endpoint_id in endpoint_ids {
            if !desired.insert(endpoint_id.clone()) {
                return Err(ConfigError(format!(
                    "duplicate dispatcher endpoint identifier {endpoint_id:?}"
                )));
            }
        }

        let mut registry = self
            .registry
            .lock()
            .map_err(|_| ConfigError("dispatcher endpoint registry is unavailable".into()))?;
        let removals = registry
            .child_ids
            .iter()
            .filter(|(id, _)| !desired.contains(*id))
            .map(|(id, child_id)| (id.clone(), *child_id))
            .collect::<Vec<_>>();
        let additions = desired
            .into_iter()
            .filter(|id| !registry.child_ids.contains_key(id))
            .map(|id| {
                let (endpoint, inventory, attestation) = build_endpoint(self.endpoint_config);
                (id, endpoint, inventory, attestation)
            })
            .collect::<Vec<_>>();

        let changes = self
            .lifecycle
            .handle
            .with_root_mut::<Root, _>(|mut root| {
                for (_, child_id) in &removals {
                    if root.remove_child(*child_id).is_none() {
                        return Err(ConfigError(
                            "dispatcher endpoint scheduler is inconsistent".into(),
                        ));
                    }
                }
                Ok(additions
                    .into_iter()
                    .map(|(id, endpoint, inventory, attestation)| {
                        let child_id = root.add_child(endpoint);
                        (id, child_id, inventory, attestation)
                    })
                    .collect::<Vec<_>>())
            })
            .ok_or_else(|| ConfigError("dispatcher runtime root is unavailable".into()))??;

        for (id, _) in removals {
            registry.child_ids.remove(&id);
            registry.gates.remove(&id);
        }
        for (id, child_id, inventory, attestation) in changes {
            let gates = EndpointGates::new(
                &id,
                inventory,
                attestation,
                self.endpoint_config.queue_capacity,
                &self.lifecycle,
            );
            registry.child_ids.insert(id.clone(), child_id);
            registry.gates.insert(id, gates);
        }
        Ok(())
    }

    /// Let every endpoint try its credentials again.
    ///
    /// Call this once at the start of each collection cycle. Credentials are
    /// reloaded per cycle, so each cycle costs a BMC with rejected credentials
    /// at most one failed login per in-flight slot.
    pub fn clear_credential_rejections(&self) {
        let Ok(registry) = self.registry.lock() else {
            return;
        };
        for gates in registry.gates.values() {
            gates.credentials.clear();
        }
    }

    /// Whether the endpoint rejected its credentials since the last
    /// [`Self::clear_credential_rejections`].
    #[must_use]
    pub fn credentials_rejected(&self, endpoint_id: &str) -> bool {
        self.registry.lock().is_ok_and(|registry| {
            registry
                .gates
                .get(endpoint_id)
                .is_some_and(|gates| gates.credentials.is_set())
        })
    }
}

struct EndpointGates {
    inventory: RequestGate,
    attestation: RequestGate,
    credentials: Arc<CredentialLatch>,
}

impl EndpointGates {
    fn new(
        endpoint_id: &str,
        inventory: Queue,
        attestation: Queue,
        capacity: usize,
        lifecycle: &Arc<DispatcherLifecycle>,
    ) -> Self {
        let credentials = Arc::new(CredentialLatch {
            endpoint_id: endpoint_id.to_string(),
            hasher: std::hash::RandomState::new(),
            rejected: Mutex::new(HashSet::new()),
        });
        let make_gate = |queue| RequestGate {
            queue,
            capacity,
            lifecycle: Arc::clone(lifecycle),
            credentials: Arc::clone(&credentials),
        };
        Self {
            inventory: make_gate(inventory),
            attestation: make_gate(attestation),
            credentials,
        }
    }
}

/// Records that a BMC rejected the collector's credentials.
///
/// Every request with rejected credentials is a failed login, and BMCs lock the
/// account after a few of them, which also locks out other clients sharing it.
/// Once set, requests to that BMC with the rejected credentials fail locally
/// until the next cycle; requests with reloaded credentials still try them.
/// All workflow gates of one endpoint share the latch.
struct CredentialLatch {
    endpoint_id: String,
    hasher: std::hash::RandomState,
    /// Fingerprints of rejected credentials, never the credentials themselves.
    rejected: Mutex<HashSet<u64>>,
}

impl CredentialLatch {
    fn rejects(&self, credentials: &BmcCredentials) -> bool {
        let fingerprint = self.hasher.hash_one(credentials);
        self.rejected
            .lock()
            .map_or(true, |rejected| rejected.contains(&fingerprint))
    }

    fn is_set(&self) -> bool {
        self.rejected
            .lock()
            .map_or(true, |rejected| !rejected.is_empty())
    }

    fn reject(&self, credentials: &BmcCredentials) {
        let fingerprint = self.hasher.hash_one(credentials);
        if self
            .rejected
            .lock()
            .is_ok_and(|mut rejected| rejected.insert(fingerprint))
        {
            tracing::warn!(
                endpoint = %self.endpoint_id,
                "BMC rejected credentials; skipping its remaining requests with them until the next cycle"
            );
        }
    }

    fn clear(&self) {
        if let Ok(mut rejected) = self.rejected.lock() {
            rejected.clear();
        }
    }
}

/// Cloneable admission handle for one BMC workflow queue.
#[derive(Clone)]
pub struct RequestGate {
    queue: Queue,
    capacity: usize,
    lifecycle: Arc<DispatcherLifecycle>,
    credentials: Arc<CredentialLatch>,
}

impl RequestGate {
    /// Admit an operation, hold its concurrency slot until completion, and
    /// return the operation's original typed result.
    ///
    /// The credential latch does not apply: use [`Self::execute_classified`]
    /// for operations that authenticate.
    pub async fn execute<T, E>(
        &self,
        operation: impl Future<Output = Result<T, E>>,
    ) -> Result<T, DispatchError<E>> {
        self.admit(operation, None, |_| FailureKind::Permanent)
            .await
    }

    /// [`Self::execute`] for an operation that authenticates with
    /// `credentials`. When `classify` reports
    /// [`FailureKind::CredentialsRejected`], later requests to this endpoint
    /// with the same credentials fail locally until the latch is cleared.
    pub async fn execute_classified<T, E>(
        &self,
        credentials: &BmcCredentials,
        operation: impl Future<Output = Result<T, E>>,
        classify: impl FnOnce(&E) -> FailureKind,
    ) -> Result<T, DispatchError<E>> {
        self.admit(operation, Some(credentials), classify).await
    }

    async fn admit<T, E>(
        &self,
        operation: impl Future<Output = Result<T, E>>,
        credentials: Option<&BmcCredentials>,
        classify: impl FnOnce(&E) -> FailureKind,
    ) -> Result<T, DispatchError<E>> {
        let rejected =
            || credentials.is_some_and(|credentials| self.credentials.rejects(credentials));
        if rejected() {
            return Err(DispatchError::CredentialsRejected);
        }
        let (started_tx, started_rx) = oneshot::channel();
        let (completed_tx, completed_rx) = oneshot::channel::<()>();
        let work: Work = Box::pin(async move {
            if started_tx.send(()).is_ok() {
                let _ = completed_rx.await;
            }
            Ok(Vec::new())
        });

        {
            let mut queue = self.queue.lock().map_err(|_| DispatchError::Unavailable)?;
            if queue.len() >= self.capacity {
                return Err(DispatchError::QueueFull);
            }
            queue.push_back(ScheduledWork::new((), work));
        }
        self.lifecycle.notify.notify_one();
        let _ = self.lifecycle.handle.with_root_mut::<Root, _>(|_| ());

        started_rx.await.map_err(|_| DispatchError::Unavailable)?;
        // A request queued before another one was rejected must not reach the
        // BMC; dropping `completed_tx` releases its slot.
        if rejected() {
            return Err(DispatchError::CredentialsRejected);
        }
        let result = operation.await;
        if let (Err(error), Some(credentials)) = (&result, credentials)
            && classify(error) == FailureKind::CredentialsRejected
        {
            self.credentials.reject(credentials);
        }
        let _ = completed_tx.send(());
        result.map_err(DispatchError::Transport)
    }

    /// [`Self::execute_classified`] for an idempotent request, which is sent
    /// again after a short backoff when it fails transiently.
    ///
    /// Each attempt is admitted through the gate, so retries respect the
    /// endpoint's rate and concurrency limits, release the slot while waiting,
    /// and stop once its credentials are rejected. Never use this for a
    /// request with side effects.
    pub async fn execute_idempotent<T, E, Fut>(
        &self,
        credentials: &BmcCredentials,
        operation: impl FnMut() -> Fut,
        classify: impl Fn(&E) -> FailureKind,
    ) -> Result<T, DispatchError<E>>
    where
        Fut: Future<Output = Result<T, E>>,
    {
        self.execute_with_retries(credentials, &RETRY_DELAYS, operation, classify)
            .await
    }

    async fn execute_with_retries<T, E, Fut>(
        &self,
        credentials: &BmcCredentials,
        delays: &[Duration],
        mut operation: impl FnMut() -> Fut,
        classify: impl Fn(&E) -> FailureKind,
    ) -> Result<T, DispatchError<E>>
    where
        Fut: Future<Output = Result<T, E>>,
    {
        let mut delays = delays.iter();
        loop {
            let result = self
                .execute_classified(credentials, operation(), &classify)
                .await;
            if let Err(DispatchError::Transport(error)) = &result
                && classify(error) == FailureKind::Transient
                && let Some(delay) = delays.next()
            {
                tokio::time::sleep(with_jitter(*delay)).await;
                continue;
            }
            return result;
        }
    }
}

/// Backoff before each retry of a transiently failed idempotent request.
///
/// Rides out a BMC that briefly times out or sheds load. A longer outage is
/// left to the caller's next collection attempt.
const RETRY_DELAYS: [Duration; 2] = [Duration::from_secs(1), Duration::from_secs(2)];

/// Add up to 25% random delay, so requests that failed together do not all
/// retry at the same instant.
fn with_jitter(delay: Duration) -> Duration {
    let random = std::collections::hash_map::RandomState::new().hash_one(());
    delay + delay.mul_f64((random % 1000) as f64 / 4000.0)
}

/// How a failed BMC request affects later requests.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum FailureKind {
    /// The BMC rejected the credentials (HTTP 401). Later requests to that BMC
    /// fail locally until the latch is cleared.
    CredentialsRejected,
    /// A timeout, a dropped connection, or an overload response (HTTP 429,
    /// 502, 503 or 504). An idempotent request may be sent again.
    Transient,
    /// Anything else, such as a 403, a 404 or an undecodable body.
    Permanent,
}

/// Classify an nv-redfish transport error.
fn classify_bmc_error<E: BmcError>(error: &E) -> FailureKind {
    match error.error_class() {
        BmcErrorClass::HttpResponse { status: 401 } => FailureKind::CredentialsRejected,
        BmcErrorClass::Transport
        | BmcErrorClass::HttpResponse {
            status: 429 | 502 | 503 | 504,
        } => FailureKind::Transient,
        _ => FailureKind::Permanent,
    }
}

fn build_endpoint(config: EndpointConfig) -> (Endpoint, Queue, Queue) {
    let inventory_queue = Arc::new(Mutex::new(VecDeque::new()));
    let attestation_queue = Arc::new(Mutex::new(VecDeque::new()));
    let mut workflows = WorkflowQueues::new();
    workflows.add_child(QueueLeaf::new(Arc::clone(&inventory_queue)));
    workflows.add_child(QueueLeaf::new(Arc::clone(&attestation_queue)));

    let costed = FixedCost::new(CostUnits::new(1), workflows);
    let rate_limited = TokenBucket::new(
        Instant::now(),
        TokenBucketConfig {
            capacity: CostUnits::new(config.burst),
            refill_amount: CostUnits::new(config.requests_per_second),
            refill_interval: Duration::from_secs(1),
        },
        costed,
    );
    (
        BoundedConcurrency::new(config.max_in_flight, rate_limited),
        inventory_queue,
        attestation_queue,
    )
}

/// Error returned by the admission layer or wrapped HTTP transport.
#[derive(Debug)]
pub enum DispatchError<E> {
    /// The workflow queue reached its configured bound.
    QueueFull,
    /// The dispatcher stopped or its queue became unavailable.
    Unavailable,
    /// The MVP adapter does not support this streaming operation.
    Unsupported,
    /// The BMC rejected the credentials earlier in this cycle, so the request
    /// was not sent.
    CredentialsRejected,
    /// The underlying HTTP client returned an error.
    Transport(E),
}

impl<E: fmt::Display> fmt::Display for DispatchError<E> {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::QueueFull => formatter.write_str("BMC dispatcher queue is full"),
            Self::Unavailable => formatter.write_str("BMC dispatcher is unavailable"),
            Self::Unsupported => formatter.write_str("BMC dispatcher does not support SSE streams"),
            Self::CredentialsRejected => formatter
                .write_str("BMC rejected credentials earlier in this cycle; request was not sent"),
            Self::Transport(error) => error.fmt(formatter),
        }
    }
}

impl<E: StdError + 'static> StdError for DispatchError<E> {
    fn source(&self) -> Option<&(dyn StdError + 'static)> {
        match self {
            Self::Transport(error) => Some(error),
            Self::QueueFull | Self::Unavailable | Self::Unsupported | Self::CredentialsRejected => {
                None
            }
        }
    }
}

impl<E: BmcError + 'static> BmcError for DispatchError<E> {
    fn response_body(&self) -> Option<&str> {
        match self {
            Self::Transport(error) => error.response_body(),
            _ => None,
        }
    }

    fn into_decode_error(self) -> Result<DecodeError, Self> {
        match self {
            Self::Transport(error) => error.into_decode_error().map_err(Self::Transport),
            other => Err(other),
        }
    }

    fn error_class(&self) -> BmcErrorClass {
        match self {
            Self::Transport(error) => error.error_class(),
            // A rejected admission failed before any response was received.
            // `Other` would let callers treat it as skippable resource data.
            Self::QueueFull | Self::Unavailable => BmcErrorClass::Transport,
            Self::Unsupported => BmcErrorClass::Other,
            // Classify like the 401 that set the latch, so callers treat it as
            // an authentication failure rather than missing resource data.
            Self::CredentialsRejected => BmcErrorClass::HttpResponse { status: 401 },
        }
    }
}

impl<E: CacheableError> CacheableError for DispatchError<E> {
    fn is_cached(&self) -> bool {
        matches!(self, Self::Transport(error) if error.is_cached())
    }

    fn cache_miss() -> Self {
        Self::Transport(E::cache_miss())
    }

    fn cache_error(reason: String) -> Self {
        Self::Transport(E::cache_error(reason))
    }
}

impl<E: RequestError> RequestError for DispatchError<E> {
    fn rejected_uri_reference(error: RejectedUriReferenceError) -> Self {
        Self::Transport(E::rejected_uri_reference(error))
    }
}

/// An `nv-redfish` HTTP client adapter that dispatches every request through
/// one [`RequestGate`].
pub struct DispatchingHttpClient<C> {
    inner: C,
    gate: RequestGate,
}

impl<C> DispatchingHttpClient<C> {
    /// Wrap an existing HTTP client with dispatcher admission.
    #[must_use]
    pub const fn new(inner: C, gate: RequestGate) -> Self {
        Self { inner, gate }
    }
}

impl<C: HttpClient> DispatchingHttpClient<C> {
    /// A 401 to a request carrying `credentials` sets the endpoint's
    /// credential latch for them.
    async fn dispatch<T>(
        &self,
        credentials: Option<&BmcCredentials>,
        request: impl Future<Output = Result<T, C::Error>>,
    ) -> Result<T, DispatchError<C::Error>> {
        self.gate
            .admit(request, credentials, classify_bmc_error)
            .await
    }

    /// [`Self::dispatch`] for a read, which is retried after a transient
    /// failure.
    async fn dispatch_read<T, Fut>(
        &self,
        credentials: &BmcCredentials,
        request: impl FnMut() -> Fut,
    ) -> Result<T, DispatchError<C::Error>>
    where
        Fut: Future<Output = Result<T, C::Error>>,
    {
        self.gate
            .execute_idempotent(credentials, request, classify_bmc_error)
            .await
    }
}

impl<C: HttpClient> HttpClient for DispatchingHttpClient<C>
where
    C::Error: Send + Sync + 'static,
{
    type Error = DispatchError<C::Error>;

    async fn get<T>(
        &self,
        url: Url,
        credentials: &BmcCredentials,
        etag: Option<ODataETag>,
        custom_headers: &HeaderMap,
        repair: &impl Repair,
    ) -> Result<T, Self::Error>
    where
        T: DeserializeOwned + Send + Sync,
    {
        self.dispatch_read(credentials, || {
            self.inner.get(
                url.clone(),
                credentials,
                etag.clone(),
                custom_headers,
                repair,
            )
        })
        .await
    }

    async fn poll<T>(
        &self,
        url: Url,
        credentials: &BmcCredentials,
        custom_headers: &HeaderMap,
    ) -> Result<ModificationResponse<T>, Self::Error>
    where
        T: DeserializeOwned + Send + Sync,
    {
        self.dispatch_read(credentials, || {
            self.inner.poll(url.clone(), credentials, custom_headers)
        })
        .await
    }

    async fn post<B, T>(
        &self,
        url: Url,
        body: &B,
        credentials: &BmcCredentials,
        custom_headers: &HeaderMap,
    ) -> Result<ModificationResponse<T>, Self::Error>
    where
        B: Serialize + Send + Sync,
        T: DeserializeOwned + Send + Sync,
    {
        self.dispatch(
            Some(credentials),
            self.inner.post(url, body, credentials, custom_headers),
        )
        .await
    }

    async fn post_session<B, T>(
        &self,
        url: Url,
        body: &B,
        custom_headers: &HeaderMap,
    ) -> Result<SessionCreateResponse<T>, Self::Error>
    where
        B: Serialize + Send + Sync,
        T: DeserializeOwned + Send + Sync,
    {
        self.dispatch(None, self.inner.post_session(url, body, custom_headers))
            .await
    }

    async fn post_multipart_update<U, V, T>(
        &self,
        url: Url,
        request: MultipartUpdateRequest<'_, U, V>,
        credentials: &BmcCredentials,
        custom_headers: &HeaderMap,
    ) -> Result<ModificationResponse<T>, Self::Error>
    where
        U: UploadReader,
        T: DeserializeOwned + Send + Sync,
        V: Serialize + Send + Sync,
    {
        self.dispatch(
            Some(credentials),
            self.inner
                .post_multipart_update(url, request, credentials, custom_headers),
        )
        .await
    }

    async fn post_http_push_uri_update<U, T>(
        &self,
        url: Url,
        request: HttpPushUriUpdateRequest<U>,
        credentials: &BmcCredentials,
        custom_headers: &HeaderMap,
    ) -> Result<ModificationResponse<T>, Self::Error>
    where
        U: UploadReader,
        T: DeserializeOwned + Send + Sync,
    {
        self.dispatch(
            Some(credentials),
            self.inner
                .post_http_push_uri_update(url, request, credentials, custom_headers),
        )
        .await
    }

    async fn patch<B, T>(
        &self,
        url: Url,
        etag: ODataETag,
        body: &B,
        credentials: &BmcCredentials,
        custom_headers: &HeaderMap,
    ) -> Result<ModificationResponse<T>, Self::Error>
    where
        B: Serialize + Send + Sync,
        T: DeserializeOwned + Send + Sync,
    {
        self.dispatch(
            Some(credentials),
            self.inner
                .patch(url, etag, body, credentials, custom_headers),
        )
        .await
    }

    async fn delete<T>(
        &self,
        url: Url,
        credentials: &BmcCredentials,
        custom_headers: &HeaderMap,
    ) -> Result<ModificationResponse<T>, Self::Error>
    where
        T: DeserializeOwned + Send + Sync,
    {
        self.dispatch(
            Some(credentials),
            self.inner.delete(url, credentials, custom_headers),
        )
        .await
    }

    async fn sse<T: Sized + for<'de> Deserialize<'de> + Send>(
        &self,
        _url: Url,
        _credentials: &BmcCredentials,
        _custom_headers: &HeaderMap,
    ) -> Result<BoxTryStream<T, Self::Error>, Self::Error> {
        Err(DispatchError::Unsupported)
    }
}

struct QueueLeaf {
    queue: Queue,
}

impl QueueLeaf {
    const fn new(queue: Queue) -> Self {
        Self { queue }
    }
}

impl Scheduler<Work> for QueueLeaf {
    type Meta = ();

    fn update_ready(&mut self, _now: Instant) -> Readiness {
        match self.queue.lock() {
            Ok(queue) if !queue.is_empty() => Readiness::ready(None),
            Ok(_) | Err(_) => Readiness::not_ready(None),
        }
    }

    fn take_next(&mut self) -> Option<ScheduledWork<Work, Self::Meta>> {
        self.queue.lock().ok()?.pop_front()
    }

    fn on_complete(&mut self, _completion: Completion<Self::Meta>) {}

    fn register_queue_event_sink(&mut self, _sink: nv_redfish_dispatcher::QueueEventSink) {}
}

#[cfg(test)]
mod tests {
    use std::convert::Infallible;
    use std::sync::Arc;
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::time::Duration;

    use nv_redfish::bmc_http::BmcCredentials;
    use nv_redfish::core::{BmcError, BmcErrorClass};

    use super::{DispatchError, Dispatcher, DispatcherConfig};

    #[derive(Debug)]
    struct UnavailableResponse;

    impl std::fmt::Display for UnavailableResponse {
        fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
            formatter.write_str("503 Service Unavailable")
        }
    }

    impl std::error::Error for UnavailableResponse {}

    impl BmcError for UnavailableResponse {
        fn error_class(&self) -> BmcErrorClass {
            BmcErrorClass::HttpResponse { status: 503 }
        }
    }

    #[test]
    fn admission_failures_classify_as_transport() {
        type Error = DispatchError<UnavailableResponse>;

        assert_eq!(
            Error::Transport(UnavailableResponse).error_class(),
            BmcErrorClass::HttpResponse { status: 503 }
        );
        assert_eq!(Error::QueueFull.error_class(), BmcErrorClass::Transport);
        assert_eq!(Error::Unavailable.error_class(), BmcErrorClass::Transport);
        assert_eq!(Error::Unsupported.error_class(), BmcErrorClass::Other);
        assert_eq!(
            Error::CredentialsRejected.error_class(),
            BmcErrorClass::HttpResponse { status: 401 },
            "a skipped request reports the authentication failure that caused it"
        );
    }

    #[derive(Debug)]
    struct StatusResponse(u16);

    impl std::fmt::Display for StatusResponse {
        fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
            write!(formatter, "HTTP {}", self.0)
        }
    }

    impl std::error::Error for StatusResponse {}

    impl BmcError for StatusResponse {
        fn error_class(&self) -> BmcErrorClass {
            BmcErrorClass::HttpResponse { status: self.0 }
        }
    }

    #[test]
    fn classifies_bmc_failures() {
        use super::FailureKind::{CredentialsRejected, Permanent, Transient};

        for (status, expected) in [
            (401, CredentialsRejected),
            (403, Permanent),
            (404, Permanent),
            (500, Permanent),
            (429, Transient),
            (502, Transient),
            (503, Transient),
            (504, Transient),
        ] {
            assert_eq!(
                super::classify_bmc_error(&StatusResponse(status)),
                expected,
                "HTTP {status}"
            );
        }
    }

    #[tokio::test]
    async fn retries_only_transient_failures_within_budget() {
        use super::FailureKind::{CredentialsRejected, Permanent, Transient};

        let delays = [Duration::from_millis(1); 2];
        // Scenario, attempt outcomes, expected attempts, expected success.
        type Case<'a> = (&'a str, &'a [Result<(), super::FailureKind>], usize, bool);
        let cases: [Case<'_>; 5] = [
            ("succeeds first", &[Ok(())], 1, true),
            ("recovers after a blip", &[Err(Transient), Ok(())], 2, true),
            (
                "gives up after the budget",
                &[Err(Transient), Err(Transient), Err(Transient), Ok(())],
                3,
                false,
            ),
            (
                "does not retry a permanent failure",
                &[Err(Permanent), Ok(())],
                1,
                false,
            ),
            (
                "does not retry a 401",
                &[Err(CredentialsRejected), Ok(())],
                1,
                false,
            ),
        ];
        for (scenario, outcomes, expected_attempts, succeeds) in cases {
            let dispatcher =
                Dispatcher::start(config(), ["bmc-1".to_string()]).expect("dispatcher");
            let gate = dispatcher.inventory_gate("bmc-1").expect("gate");
            let attempts = AtomicUsize::new(0);
            let result = gate
                .execute_with_retries(
                    &BmcCredentials::new("user".into(), "password".into()),
                    &delays,
                    || {
                        let attempt = attempts.fetch_add(1, Ordering::SeqCst);
                        std::future::ready(outcomes[attempt])
                    },
                    |kind: &super::FailureKind| *kind,
                )
                .await;
            assert_eq!(
                attempts.load(Ordering::SeqCst),
                expected_attempts,
                "{scenario}"
            );
            assert_eq!(result.is_ok(), succeeds, "{scenario}");
        }
    }

    #[tokio::test]
    async fn credential_rejection_stops_requests_with_those_credentials_until_cleared() {
        let old = BmcCredentials::new("user".into(), "old".into());
        let new = BmcCredentials::new("user".into(), "new".into());
        let dispatcher = Dispatcher::start(config(), ["bmc-1".to_string(), "bmc-2".to_string()])
            .expect("dispatcher");
        let inventory = dispatcher.inventory_gate("bmc-1").expect("inventory gate");
        let attestation = dispatcher
            .attestation_gate("bmc-1")
            .expect("attestation gate");
        let sent = Arc::new(AtomicUsize::new(0));
        let (started_tx, started_rx) = tokio::sync::oneshot::channel();
        let (release_tx, release_rx) = tokio::sync::oneshot::channel();

        let rejected_gate = inventory.clone();
        let rejected = tokio::spawn(async move {
            rejected_gate
                .execute_classified(
                    &BmcCredentials::new("user".into(), "old".into()),
                    async move {
                        let _ = started_tx.send(());
                        let _ = release_rx.await;
                        Err::<(), _>(StatusResponse(401))
                    },
                    super::classify_bmc_error,
                )
                .await
        });
        started_rx.await.expect("rejected request started");

        // Queued behind the 401 on the other workflow, before the latch is set.
        let queued_sent = Arc::clone(&sent);
        let queued_gate = attestation.clone();
        let queued_credentials = old.clone();
        let queued = tokio::spawn(async move {
            queued_gate
                .execute_classified(
                    &queued_credentials,
                    async move {
                        queued_sent.fetch_add(1, Ordering::SeqCst);
                        Ok::<_, Infallible>(())
                    },
                    |_| super::FailureKind::Permanent,
                )
                .await
        });
        for _ in 0..100 {
            if attestation.queue.lock().expect("queue").len() == 1 {
                break;
            }
            tokio::task::yield_now().await;
        }
        assert_eq!(attestation.queue.lock().expect("queue").len(), 1);

        let _ = release_tx.send(());
        assert!(matches!(
            rejected.await.expect("task"),
            Err(DispatchError::Transport(StatusResponse(401)))
        ));
        assert!(matches!(
            queued.await.expect("task"),
            Err(DispatchError::CredentialsRejected)
        ));
        let send = |credentials| {
            inventory.execute_classified(
                credentials,
                async {
                    sent.fetch_add(1, Ordering::SeqCst);
                    Ok::<_, Infallible>(())
                },
                |_| super::FailureKind::Permanent,
            )
        };
        assert!(matches!(
            send(&old).await,
            Err(DispatchError::CredentialsRejected)
        ));
        assert_eq!(sent.load(Ordering::SeqCst), 0, "no request reached bmc-1");
        send(&new).await.expect("reloaded credentials are tried");
        assert_eq!(sent.load(Ordering::SeqCst), 1);
        assert!(dispatcher.credentials_rejected("bmc-1"));

        dispatcher
            .inventory_gate("bmc-2")
            .expect("other endpoint")
            .execute(async { Ok::<_, Infallible>(()) })
            .await
            .expect("other endpoints are unaffected");

        dispatcher.clear_credential_rejections();
        assert!(!dispatcher.credentials_rejected("bmc-1"));
        send(&old)
            .await
            .expect("the next cycle tries the credentials again");
    }

    fn config() -> DispatcherConfig {
        DispatcherConfig {
            global_max_in_flight: 2,
            bmc_max_in_flight: 1,
            bmc_requests_per_second: 100,
            bmc_burst: 100,
            workflow_queue_capacity: 8,
        }
    }

    #[tokio::test]
    async fn enforces_per_endpoint_concurrency() {
        let dispatcher = Dispatcher::start(config(), ["bmc-1".to_string()]).expect("dispatcher");
        let gate = dispatcher.inventory_gate("bmc-1").expect("gate");
        let active = Arc::new(AtomicUsize::new(0));
        let peak = Arc::new(AtomicUsize::new(0));

        let operations = (0..3).map(|_| {
            let gate = gate.clone();
            let active = Arc::clone(&active);
            let peak = Arc::clone(&peak);
            tokio::spawn(async move {
                gate.execute(async {
                    let now = active.fetch_add(1, Ordering::SeqCst) + 1;
                    peak.fetch_max(now, Ordering::SeqCst);
                    tokio::time::sleep(Duration::from_millis(10)).await;
                    active.fetch_sub(1, Ordering::SeqCst);
                    Ok::<_, Infallible>(())
                })
                .await
            })
        });

        for operation in operations {
            operation.await.expect("task").expect("operation");
        }
        assert_eq!(peak.load(Ordering::SeqCst), 1);
    }

    #[tokio::test]
    async fn keeps_inventory_and_attestation_queues_independent() {
        let dispatcher = Dispatcher::start(config(), ["bmc-1".to_string()]).expect("dispatcher");
        assert!(dispatcher.inventory_gate("bmc-1").is_some());
        assert!(dispatcher.attestation_gate("bmc-1").is_some());
        assert!(dispatcher.inventory_gate("missing").is_none());
    }

    #[tokio::test]
    async fn shares_endpoint_limit_across_workflows() {
        let dispatcher = Dispatcher::start(config(), ["bmc-1".to_string()]).expect("dispatcher");
        let inventory = dispatcher.inventory_gate("bmc-1").expect("inventory gate");
        let attestation = dispatcher
            .attestation_gate("bmc-1")
            .expect("attestation gate");
        let active = Arc::new(AtomicUsize::new(0));
        let peak = Arc::new(AtomicUsize::new(0));

        let run = |gate: super::RequestGate| {
            let active = Arc::clone(&active);
            let peak = Arc::clone(&peak);
            tokio::spawn(async move {
                gate.execute(async {
                    let now = active.fetch_add(1, Ordering::SeqCst) + 1;
                    peak.fetch_max(now, Ordering::SeqCst);
                    tokio::time::sleep(Duration::from_millis(10)).await;
                    active.fetch_sub(1, Ordering::SeqCst);
                    Ok::<_, Infallible>(())
                })
                .await
            })
        };

        let inventory_task = run(inventory);
        let attestation_task = run(attestation);
        inventory_task.await.expect("task").expect("inventory");
        attestation_task.await.expect("task").expect("attestation");
        assert_eq!(peak.load(Ordering::SeqCst), 1);
    }

    #[tokio::test]
    async fn cloned_gate_keeps_dispatcher_running() {
        let dispatcher = Dispatcher::start(config(), ["bmc-1".to_string()]).expect("dispatcher");
        let gate = dispatcher.inventory_gate("bmc-1").expect("gate");
        drop(dispatcher);

        tokio::time::timeout(
            Duration::from_millis(250),
            gate.execute(async { Ok::<_, Infallible>(()) }),
        )
        .await
        .expect("gate must not hang after its dispatcher owner is dropped")
        .expect("operation");
    }

    #[tokio::test]
    async fn rejects_work_when_workflow_queue_is_full() {
        let mut limited = config();
        limited.global_max_in_flight = 1;
        limited.workflow_queue_capacity = 1;
        let dispatcher = Dispatcher::start(limited, ["bmc-1".to_string()]).expect("dispatcher");
        let gate = dispatcher.inventory_gate("bmc-1").expect("gate");
        let (started_tx, started_rx) = tokio::sync::oneshot::channel();
        let (release_tx, release_rx) = tokio::sync::oneshot::channel();

        let first_gate = gate.clone();
        let first = tokio::spawn(async move {
            first_gate
                .execute(async move {
                    let _ = started_tx.send(());
                    let _ = release_rx.await;
                    Ok::<_, Infallible>(())
                })
                .await
        });
        started_rx.await.expect("first request started");

        let second_gate = gate.clone();
        let second =
            tokio::spawn(
                async move { second_gate.execute(async { Ok::<_, Infallible>(()) }).await },
            );
        for _ in 0..100 {
            if gate.queue.lock().expect("queue").len() == 1 {
                break;
            }
            tokio::task::yield_now().await;
        }
        assert_eq!(gate.queue.lock().expect("queue").len(), 1);

        let rejected = gate.execute(async { Ok::<_, Infallible>(()) }).await;
        assert!(matches!(rejected, Err(DispatchError::QueueFull)));

        let _ = release_tx.send(());
        first.await.expect("first task").expect("first request");
        second.await.expect("second task").expect("second request");
    }

    #[tokio::test]
    async fn synchronizes_endpoint_membership() {
        let dispatcher = Dispatcher::start(config(), ["bmc-1".to_string()]).expect("dispatcher");

        dispatcher
            .sync_endpoints(["bmc-2".to_string()])
            .expect("sync endpoints");

        assert!(dispatcher.inventory_gate("bmc-1").is_none());
        let gate = dispatcher.inventory_gate("bmc-2").expect("new endpoint");
        gate.execute(async { Ok::<_, Infallible>(()) })
            .await
            .expect("new endpoint request");
    }
}
