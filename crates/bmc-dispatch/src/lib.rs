//! Collector-owned admission control for BMC HTTP requests.

use std::collections::{HashMap, VecDeque};
use std::convert::Infallible;
use std::error::Error as StdError;
use std::fmt;
use std::future::Future;
use std::num::{NonZeroU32, NonZeroUsize};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use http::HeaderMap;
use nv_redfish::bmc_http::{
    BmcCredentials, CacheableError, HttpClient, MultipartUpdateRequest, RejectedUriReferenceError,
    RequestError,
};
use nv_redfish::core::{
    BoxTryStream, ModificationResponse, ODataETag, SessionCreateResponse, UploadReader,
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
    gates: HashMap<String, EndpointGates>,
    handle: Handle,
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

        let notify = Arc::new(Notify::new());
        let mut root = Root::new();
        let mut pending_gates = HashMap::new();
        for endpoint_id in endpoint_ids {
            if pending_gates.contains_key(&endpoint_id) {
                return Err(ConfigError(format!(
                    "duplicate dispatcher endpoint identifier {endpoint_id:?}"
                )));
            }

            let inventory_queue = Arc::new(Mutex::new(VecDeque::new()));
            let attestation_queue = Arc::new(Mutex::new(VecDeque::new()));
            let mut workflows = WorkflowQueues::new();
            workflows.add_child(QueueLeaf::new(Arc::clone(&inventory_queue)));
            workflows.add_child(QueueLeaf::new(Arc::clone(&attestation_queue)));

            let costed = FixedCost::new(CostUnits::new(1), workflows);
            let rate_limited = TokenBucket::new(
                Instant::now(),
                TokenBucketConfig {
                    capacity: CostUnits::new(config.bmc_burst),
                    refill_amount: CostUnits::new(config.bmc_requests_per_second),
                    refill_interval: Duration::from_secs(1),
                },
                costed,
            );
            root.add_child(BoundedConcurrency::new(bmc_max_in_flight, rate_limited));
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

        let gates = pending_gates
            .into_iter()
            .map(|(id, (inventory, attestation))| {
                let make_gate = |queue| RequestGate {
                    queue,
                    capacity: config.workflow_queue_capacity,
                    handle: handle.clone(),
                    notify: Arc::clone(&notify),
                };
                (
                    id,
                    EndpointGates {
                        inventory: make_gate(inventory),
                        attestation: make_gate(attestation),
                    },
                )
            })
            .collect();

        Ok(Self { gates, handle })
    }

    /// Return the inventory admission gate for an endpoint.
    #[must_use]
    pub fn inventory_gate(&self, endpoint_id: &str) -> Option<RequestGate> {
        self.gates
            .get(endpoint_id)
            .map(|gates| gates.inventory.clone())
    }

    /// Return the attestation admission gate for an endpoint.
    #[must_use]
    pub fn attestation_gate(&self, endpoint_id: &str) -> Option<RequestGate> {
        self.gates
            .get(endpoint_id)
            .map(|gates| gates.attestation.clone())
    }
}

impl Drop for Dispatcher {
    fn drop(&mut self) {
        self.handle.graceful_shutdown();
    }
}

struct EndpointGates {
    inventory: RequestGate,
    attestation: RequestGate,
}

/// Cloneable admission handle for one BMC workflow queue.
#[derive(Clone)]
pub struct RequestGate {
    queue: Queue,
    capacity: usize,
    handle: Handle,
    notify: Arc<Notify>,
}

impl RequestGate {
    /// Admit an operation, hold its concurrency slot until completion, and
    /// return the operation's original typed result.
    pub async fn execute<T, E>(
        &self,
        operation: impl Future<Output = Result<T, E>>,
    ) -> Result<T, DispatchError<E>> {
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
        self.notify.notify_one();
        let _ = self.handle.with_root_mut::<Root, _>(|_| ());

        started_rx.await.map_err(|_| DispatchError::Unavailable)?;
        let result = operation.await.map_err(DispatchError::Transport);
        let _ = completed_tx.send(());
        result
    }
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
    /// The underlying HTTP client returned an error.
    Transport(E),
}

impl<E: fmt::Display> fmt::Display for DispatchError<E> {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::QueueFull => formatter.write_str("BMC dispatcher queue is full"),
            Self::Unavailable => formatter.write_str("BMC dispatcher is unavailable"),
            Self::Unsupported => formatter.write_str("BMC dispatcher does not support SSE streams"),
            Self::Transport(error) => error.fmt(formatter),
        }
    }
}

impl<E: StdError + 'static> StdError for DispatchError<E> {
    fn source(&self) -> Option<&(dyn StdError + 'static)> {
        match self {
            Self::Transport(error) => Some(error),
            Self::QueueFull | Self::Unavailable | Self::Unsupported => None,
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
    ) -> Result<T, Self::Error>
    where
        T: DeserializeOwned + Send + Sync,
    {
        self.gate
            .execute(self.inner.get(url, credentials, etag, custom_headers))
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
        self.gate
            .execute(self.inner.post(url, body, credentials, custom_headers))
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
        self.gate
            .execute(self.inner.post_session(url, body, custom_headers))
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
        self.gate
            .execute(
                self.inner
                    .post_multipart_update(url, request, credentials, custom_headers),
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
        self.gate
            .execute(
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
        self.gate
            .execute(self.inner.delete(url, credentials, custom_headers))
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
}

#[cfg(test)]
mod tests {
    use std::convert::Infallible;
    use std::sync::Arc;
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::time::Duration;

    use super::{Dispatcher, DispatcherConfig};

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
}
