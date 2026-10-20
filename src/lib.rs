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

mod backend;

use std::error::Error;
use std::fmt;
use std::io;
use std::pin::Pin;
use std::sync::Arc;
use std::time::Duration;

use bmc_attestation::collector;
use bmc_attestation::config as attestation_config;
use bmc_attestation::models::BatchAttestation;
use bmc_attestation::sink::BackendSink as AttestationBackendSink;
use bmc_dispatch::{Dispatcher, DispatcherConfig};
use bmc_inventory::config;
use bmc_inventory::models::{InventoryUuid, NodeInventory};
use bmc_inventory::sink::{
    BackendBatchOutcome, BackendNodeOutcome, BackendSink as InventoryBackendSink, CollectorConfig,
    CollectorRuntimeConfig, InventorySink, JsonSink, NicoHealthConfig, OtelGatewayConfig,
    TableSink,
};
use bmc_inventory::source::{InventorySource, LatencyProbe, RedfishSource, TcpProbeConfig};
use futures::stream::{self, Stream, StreamExt};
use tokio::sync::watch;

use backend::{BackendEndpoints, BackendSession, is_auth_failure, required_backend_option};

pub const DEFAULT_REDFISH_TIMEOUT_SECONDS: u64 = 30;
pub const DEFAULT_TARGET_CONCURRENCY: usize = 64;
pub const DEFAULT_BMC_MAX_IN_FLIGHT: u32 = 4;
pub const DEFAULT_BMC_REQUESTS_PER_SECOND: u64 = 10;
pub const DEFAULT_BMC_BURST: u64 = 10;
pub const DEFAULT_INVENTORY_INTERVAL_SECONDS: u64 = 24 * 60 * 60;
const WORKFLOW_QUEUE_CAPACITY: usize = 256;
const INVENTORY_EXPORT_FLUSH_INTERVAL: Duration = Duration::from_secs(30);

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum InventoryFormat {
    Table,
    Json,
}

#[derive(Clone, Debug)]
pub struct InventoryOptions {
    pub config: String,
    pub insecure_tls: bool,
    pub detail: bool,
    pub format: InventoryFormat,
    pub target_concurrency: usize,
    pub bmc_max_in_flight: u32,
    pub bmc_requests_per_second: u64,
    pub bmc_burst: u64,
    pub timeout_seconds: u64,
    pub latency_probe: bool,
}

#[derive(Clone, Debug)]
pub struct RuntimeOptions {
    pub inventory: InventoryOptions,
    pub backend_url: Option<String>,
    pub agent_token: Option<String>,
    pub interval_seconds: u64,
}

impl RuntimeOptions {
    fn runtime_config(&self) -> CollectorRuntimeConfig {
        CollectorRuntimeConfig {
            inventory_interval_seconds: self.interval_seconds.max(1),
            target_concurrency: self.inventory.target_concurrency.max(1),
            bmc_max_in_flight: self.inventory.bmc_max_in_flight,
            bmc_requests_per_second: self.inventory.bmc_requests_per_second,
            bmc_burst: self.inventory.bmc_burst,
            timeout_seconds: self.inventory.timeout_seconds,
            insecure_tls: self.inventory.insecure_tls,
            latency_probe: self.inventory.latency_probe,
            log_level: std::env::var("RUST_LOG").ok(),
        }
    }
}

fn parse_optional_bool(
    name: &str,
    value: Result<String, std::env::VarError>,
) -> Result<Option<bool>, Box<dyn Error>> {
    match value {
        Err(std::env::VarError::NotPresent) => Ok(None),
        Ok(value) if value == "true" => Ok(Some(true)),
        Ok(value) if value == "false" => Ok(Some(false)),
        _ => Err(format!("{name} must be true or false").into()),
    }
}

#[derive(Clone)]
struct Shutdown(watch::Receiver<bool>);

impl Shutdown {
    fn is_requested(&self) -> bool {
        *self.0.borrow()
    }

    async fn wait(&mut self) {
        let _ = self.0.wait_for(|requested| *requested).await;
    }
}

async fn shutdown_signal() -> io::Result<()> {
    #[cfg(unix)]
    {
        let mut terminate =
            tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate())?;
        tokio::select! {
            result = tokio::signal::ctrl_c() => result,
            _ = terminate.recv() => Ok(()),
        }
    }
    #[cfg(not(unix))]
    tokio::signal::ctrl_c().await
}

pub async fn run_runtime(options: RuntimeOptions) -> Result<(), Box<dyn Error>> {
    let (sender, receiver) = watch::channel(false);
    let runtime = run_runtime_until_shutdown(options, Shutdown(receiver));
    tokio::pin!(runtime);
    tokio::select! {
        biased;
        signal = shutdown_signal() => {
            signal?;
            tracing::info!("shutdown requested; stopping collection and draining inventory uploads");
            sender.send_replace(true);
            runtime.await
        }
        result = &mut runtime => result,
    }
}

async fn run_runtime_until_shutdown(
    options: RuntimeOptions,
    mut shutdown: Shutdown,
) -> Result<(), Box<dyn Error>> {
    let collector_config = CollectorConfig {
        runtime_config: options.runtime_config(),
        nico_health: parse_optional_bool(
            "FLEETINT_NICO_ENABLED",
            std::env::var("FLEETINT_NICO_ENABLED"),
        )?
        .map(|enabled| NicoHealthConfig::from_iter([("enabled".to_string(), enabled.into())])),
        otel_gateway: parse_optional_bool(
            "FLEETINT_OTEL_GATEWAY_ENABLED",
            std::env::var("FLEETINT_OTEL_GATEWAY_ENABLED"),
        )?
        .map(|enabled| OtelGatewayConfig::from_iter([("enabled".to_string(), enabled.into())])),
    };
    let configured_targets = config::load_targets(&options.inventory.config)?;
    let dispatcher = Arc::new(start_dispatcher(&options.inventory, &configured_targets)?);
    let backend_url = required_backend_option(options.backend_url, "backend URL")?;
    let endpoints = BackendEndpoints::resolve(&backend_url)?;
    let agent_token = required_backend_option(options.agent_token, "agent token")?;
    // The session owns the JWT lifecycle: enrolled_at is captured at enrollment and
    // refreshed whenever the agent re-enrolls (e.g. after a JWT rejection).
    let mut session = tokio::select! {
        biased;
        () = shutdown.wait() => return Ok(()),
        session = BackendSession::enroll(endpoints.enroll_endpoint.clone(), agent_token) => session?,
    };
    let interval = std::time::Duration::from_secs(options.interval_seconds.max(1));

    let attestation_settings = {
        let mut s = attestation_config::defaults::attestation_config();
        s.insecure_tls = options.inventory.insecure_tls;
        s.concurrency = options.inventory.target_concurrency;
        s.redfish_timeout = std::time::Duration::from_secs(options.inventory.timeout_seconds);
        s
    };

    loop {
        if shutdown.is_requested() {
            return Ok(());
        }
        let started_at = std::time::Instant::now();

        // Phase 1: Collect and upload inventory
        match collect_and_upload_inventory(
            options.inventory.clone(),
            collector_config.clone(),
            endpoints.oob_nodes_endpoint.clone(),
            &mut session,
            Arc::clone(&dispatcher),
            shutdown.clone(),
        )
        .await
        {
            Ok(outcome) => {
                if shutdown.is_requested() {
                    return Ok(());
                }
                if outcome.failed_uploads > 0 {
                    tracing::warn!(
                        elapsed_seconds = started_at.elapsed().as_secs_f64(),
                        resolved_node_count = outcome.nodes.len(),
                        failed_upload_count = outcome.failed_uploads,
                        "inventory backend sync completed with failed node exports"
                    );
                } else {
                    tracing::info!(
                        elapsed_seconds = started_at.elapsed().as_secs_f64(),
                        resolved_node_count = outcome.nodes.len(),
                        "inventory backend sync completed"
                    );
                }
                // Phase 2: Attest only nodes whose inventory export succeeded and
                // produced a UUID known to the backend.
                match build_attestation_targets(&options.inventory, &outcome.nodes) {
                    Err(err) => tracing::error!(
                        error = %err,
                        "failed to load targets for attestation; skipping attestation"
                    ),
                    Ok(attestation_targets) if attestation_targets.is_empty() => {
                        tracing::warn!("no nodes with resolved UUID; skipping attestation");
                    }
                    Ok(attestation_targets) => {
                        match collect_and_upload_attestation(
                            &attestation_targets,
                            &attestation_settings,
                            &endpoints,
                            &mut session,
                            dispatcher.as_ref(),
                            shutdown.clone(),
                        )
                        .await
                        {
                            Ok(()) => tracing::info!(
                                elapsed_seconds = started_at.elapsed().as_secs_f64(),
                                node_count = attestation_targets.len(),
                                "attestation backend sync completed"
                            ),
                            Err(err) => tracing::error!(
                                elapsed_seconds = started_at.elapsed().as_secs_f64(),
                                error = %err,
                                "attestation backend sync failed"
                            ),
                        }
                    }
                }
            }
            Err(err) => tracing::error!(
                elapsed_seconds = started_at.elapsed().as_secs_f64(),
                next_attempt_seconds = interval.as_secs(),
                error = %err,
                "inventory backend sync failed"
            ),
        }

        tracing::info!(
            next_sync_seconds = interval.as_secs(),
            "sleeping until next cycle"
        );
        tokio::select! {
            _ = tokio::time::sleep(interval) => {}
            () = shutdown.wait() => return Ok(()),
        }
    }
}

fn build_attestation_targets(
    options: &InventoryOptions,
    nodes: &[ResolvedNode],
) -> Result<Vec<attestation_config::Target>, Box<dyn Error>> {
    let targets = config::load_targets(&options.config)?;

    Ok(targets
        .into_iter()
        .filter_map(|target| {
            let mac = target.mac.to_string();
            let uuid = nodes.iter().find(|node| node.mac == mac)?.node_uuid.clone();
            Some(attestation_config::Target {
                name: mac,
                node_uuid: uuid,
                address: target.address,
                username: target.username,
                password: target.password,
            })
        })
        .collect())
}

async fn collect_and_upload_attestation(
    targets: &[attestation_config::Target],
    attestation_settings: &attestation_config::AttestationConfig,
    endpoints: &BackendEndpoints,
    session: &mut BackendSession,
    dispatcher: &Dispatcher,
    mut shutdown: Shutdown,
) -> Result<(), Box<dyn Error>> {
    let collect = |jwt: String| async move {
        collect_attestation(targets, attestation_settings, endpoints, &jwt, dispatcher).await
    };
    let result = tokio::select! {
        biased;
        () = shutdown.wait() => return Ok(()),
        result = collect(session.jwt().to_string()) => result,
    };
    let outcome = match result {
        Ok(outcome) => outcome,
        Err(err) if is_auth_failure(err.as_ref()) => {
            tracing::warn!(error = %err, "backend JWT rejected during nonce collection; enrolling again before retry");
            session.reenroll().await?;
            tokio::select! {
                biased;
                () = shutdown.wait() => return Ok(()),
                result = collect(session.jwt().to_string()) => result?,
            }
        }
        Err(err) => return Err(err),
    };
    let summary = summarize_attestation_collection(&outcome.batch);
    if summary.failed > 0 || summary.component_errors > 0 {
        tracing::warn!(
            node_count = summary.total,
            successful_count = summary.successful,
            failed_count = summary.failed,
            evidence_count = summary.evidences,
            component_error_count = summary.component_errors,
            "attestation collection completed with failures"
        );
    } else {
        tracing::info!(
            node_count = summary.total,
            successful_count = summary.successful,
            failed_count = summary.failed,
            evidence_count = summary.evidences,
            component_error_count = summary.component_errors,
            "attestation collection completed"
        );
    }
    // Carry any rotated token forward so the upload and subsequent cycles authenticate
    // with the freshest JWT instead of the one we started the cycle with.
    if let Some(token) = outcome.refreshed_jwt {
        session.set_jwt(token);
    }

    upload_attestation_batches(
        endpoints,
        session,
        &outcome.batch,
        attestation_settings.redfish_timeout,
        attestation_settings.concurrency,
        &shutdown,
    )
    .await
}

async fn upload_attestation_batches(
    endpoints: &BackendEndpoints,
    session: &mut BackendSession,
    batch: &BatchAttestation,
    timeout: Duration,
    batch_size: usize,
    shutdown: &Shutdown,
) -> Result<(), Box<dyn Error>> {
    let batches = split_attestation_batch(batch, batch_size);
    let batch_count = batches.len();
    let mut failures = Vec::new();

    for (index, batch) in batches.into_iter().enumerate() {
        if shutdown.is_requested() {
            break;
        }
        let node_count = batch.results_by_node_uuid.len();
        let mut result = upload_attestation(endpoints, session.jwt(), &batch, timeout).await;
        if let Err(err) = &result
            && is_auth_failure(err.as_ref())
        {
            tracing::warn!(
                error = %err,
                batch_index = index + 1,
                batch_count,
                "backend JWT rejected on attestation upload; enrolling again before retry"
            );
            session.reenroll().await?;
            result = upload_attestation(endpoints, session.jwt(), &batch, timeout).await;
        }

        match result {
            Ok(()) => tracing::info!(
                batch_index = index + 1,
                batch_count,
                node_count,
                "attestation mini-batch export completed"
            ),
            Err(err) => {
                tracing::error!(
                    batch_index = index + 1,
                    batch_count,
                    node_count,
                    error = %err,
                    "attestation mini-batch export failed"
                );
                failures.push(err.to_string());
            }
        }
    }

    if failures.is_empty() {
        Ok(())
    } else {
        Err(format!(
            "{} attestation mini-batch export(s) failed; first error: {}",
            failures.len(),
            failures[0]
        )
        .into())
    }
}

fn split_attestation_batch(batch: &BatchAttestation, batch_size: usize) -> Vec<BatchAttestation> {
    let batch_size = batch_size.max(1);
    let results = batch
        .results_by_node_uuid
        .values()
        .cloned()
        .collect::<Vec<_>>();
    results
        .chunks(batch_size)
        .map(|chunk| BatchAttestation::new(batch.collected_at.clone(), chunk.to_vec()))
        .collect()
}

async fn collect_attestation(
    targets: &[attestation_config::Target],
    attestation_settings: &attestation_config::AttestationConfig,
    endpoints: &BackendEndpoints,
    jwt: &str,
    dispatcher: &Dispatcher,
) -> Result<collector::CollectionOutcome, Box<dyn Error>> {
    collector::collect(
        targets,
        attestation_settings,
        &endpoints.base_url,
        &endpoints.nonce_path,
        &endpoints.attestation_path,
        jwt,
        dispatcher,
    )
    .await
}

async fn upload_attestation(
    endpoints: &BackendEndpoints,
    jwt: &str,
    batch: &BatchAttestation,
    timeout: std::time::Duration,
) -> Result<(), Box<dyn Error>> {
    let sink = AttestationBackendSink::new(
        &endpoints.base_url,
        &endpoints.nonce_path,
        &endpoints.attestation_path,
        jwt,
        timeout,
    )?;
    sink.write(batch).await
}

pub async fn run_inventory(options: InventoryOptions) -> Result<(), Box<dyn Error>> {
    let collector_config = config::load_config(&options.config)?;
    let dispatcher = Arc::new(start_dispatcher(&options, &collector_config.targets)?);
    let inventories =
        collect_inventory_targets(&options, collector_config.targets, dispatcher).await?;

    match options.format {
        InventoryFormat::Table => TableSink::new(options.detail).write(&inventories)?,
        InventoryFormat::Json => JsonSink.write(&inventories)?,
    }
    Ok(())
}

async fn collect_and_upload_inventory(
    options: InventoryOptions,
    collector_config: CollectorConfig,
    endpoint: String,
    session: &mut BackendSession,
    dispatcher: Arc<Dispatcher>,
    shutdown: Shutdown,
) -> Result<InventorySyncOutcome, Box<dyn Error>> {
    let target_config = config::load_config(&options.config)?;
    dispatcher.sync_endpoints(
        target_config
            .targets
            .iter()
            .map(|target| target.mac.to_string()),
    )?;
    let mut backend_sink =
        InventoryBackendSink::new(endpoint, session.jwt())?.with_collector_config(collector_config);
    let timeout = Duration::from_secs(options.timeout_seconds);
    let target_concurrency = options.target_concurrency.max(1);
    let source = RedfishSource::new(options.insecure_tls, timeout, dispatcher)?.with_latency_probe(
        LatencyProbe::for_run(options.latency_probe, TcpProbeConfig::default()).await,
    );
    let channel_capacity = target_concurrency.saturating_mul(2).max(1);
    let (tx, mut rx) = tokio::sync::mpsc::channel(channel_capacity);
    // JoinSet aborts the producer if an upload error exits this scope early.
    let mut producer = tokio::task::JoinSet::new();
    producer.spawn(async move {
        let collections = inventory_collections(
            &source,
            target_config.targets,
            target_concurrency,
            CollectionOrder::Completion,
        );
        forward_inventory(collections, tx, shutdown).await
    });

    let mut inventory_vec = Vec::new();
    let mut summary = InventoryCollectionSummary::default();
    let mut failed_uploads = 0;
    loop {
        let batch =
            receive_inventory_batch(&mut rx, target_concurrency, INVENTORY_EXPORT_FLUSH_INTERVAL)
                .await;
        if batch.is_empty() {
            break;
        }
        summary.add(summarize_inventory_collection(&batch));
        let exportable_count = batch.len();
        let (uploaded, upload_result) =
            upload_inventory_batch(&mut backend_sink, session, batch).await?;
        match upload_result {
            Ok(outcome) => {
                let outcomes = (0..uploaded.len()).map(|index| outcome.item_outcome(index));
                let (accepted, failures) = partition_uploaded_inventory(uploaded, outcomes);
                failed_uploads += failures.len();
                for failure in &failures {
                    match failure {
                        InventoryUploadFailure::Rejected {
                            mac,
                            status,
                            message,
                        } => tracing::warn!(
                            mac,
                            status,
                            message = message.as_deref().unwrap_or(""),
                            "backend rejected inventory node"
                        ),
                        InventoryUploadFailure::MissingResult { mac } => tracing::warn!(
                            mac,
                            "backend returned success without a result row for inventory node"
                        ),
                    }
                }
                if !failures.is_empty() {
                    tracing::warn!(
                        exportable_count,
                        resolved_node_count = accepted.len(),
                        failed_count = failures.len(),
                        "inventory mini-batch export completed with failed nodes"
                    );
                } else {
                    tracing::info!(exportable_count, "inventory mini-batch export completed");
                }
                inventory_vec.extend(accepted);
            }
            Err(err) => {
                failed_uploads += exportable_count;
                tracing::error!(
                    exportable_count,
                    error = %err,
                    "inventory mini-batch export failed"
                );
            }
        }
    }
    producer
        .join_next()
        .await
        .expect("inventory producer was started")
        .map_err(|err| format!("inventory collection task failed: {err}"))?
        .map_err(|err| -> Box<dyn Error> { err.into() })?;

    if summary.partial > 0 || summary.failed > 0 {
        tracing::warn!(
            target_count = summary.total,
            successful_count = summary.successful,
            partial_count = summary.partial,
            failed_count = summary.failed,
            "inventory collection completed with partial or failed targets"
        );
    } else {
        tracing::info!(
            target_count = summary.total,
            successful_count = summary.successful,
            partial_count = summary.partial,
            failed_count = summary.failed,
            "inventory collection completed"
        );
    }
    Ok(InventorySyncOutcome {
        nodes: inventory_vec,
        failed_uploads,
    })
}

async fn forward_inventory(
    mut collections: impl Stream<Item = NodeInventory> + Unpin,
    sender: tokio::sync::mpsc::Sender<NodeInventory>,
    mut shutdown: Shutdown,
) -> Result<(), &'static str> {
    loop {
        let inventory = tokio::select! {
            biased;
            () = shutdown.wait() => break,
            inventory = collections.next() => match inventory {
                Some(inventory) => inventory,
                None => break,
            },
        };
        tokio::select! {
            biased;
            () = shutdown.wait() => break,
            result = sender.send(inventory) => {
                result.map_err(|_| "inventory export worker stopped before collection completed")?;
            }
        }
    }
    Ok(())
}

struct InventorySyncOutcome {
    nodes: Vec<ResolvedNode>,
    failed_uploads: usize,
}

#[derive(Debug, Default, Eq, PartialEq)]
struct InventoryCollectionSummary {
    total: usize,
    successful: usize,
    partial: usize,
    failed: usize,
}

impl InventoryCollectionSummary {
    fn add(&mut self, other: Self) {
        self.total += other.total;
        self.successful += other.successful;
        self.partial += other.partial;
        self.failed += other.failed;
    }
}

fn summarize_inventory_collection(inventories: &[NodeInventory]) -> InventoryCollectionSummary {
    let failed = inventories
        .iter()
        .filter(|inventory| inventory.target_error.is_some())
        .count();
    let partial = inventories
        .iter()
        .filter(|inventory| inventory.target_error.is_none() && !inventory.domain_errors.is_empty())
        .count();
    InventoryCollectionSummary {
        total: inventories.len(),
        successful: inventories.len().saturating_sub(failed + partial),
        partial,
        failed,
    }
}

#[derive(Debug, Eq, PartialEq)]
struct AttestationCollectionSummary {
    total: usize,
    successful: usize,
    failed: usize,
    evidences: usize,
    component_errors: usize,
}

fn summarize_attestation_collection(batch: &BatchAttestation) -> AttestationCollectionSummary {
    let successful = batch
        .results_by_node_uuid
        .values()
        .filter(|attestation| attestation.success)
        .count();
    AttestationCollectionSummary {
        total: batch.results_by_node_uuid.len(),
        successful,
        failed: batch.results_by_node_uuid.len().saturating_sub(successful),
        evidences: batch
            .results_by_node_uuid
            .values()
            .map(|attestation| attestation.evidences.len())
            .sum(),
        component_errors: batch
            .results_by_node_uuid
            .values()
            .map(|attestation| attestation.component_errors.len())
            .sum(),
    }
}

fn stamp_enrolled_at(inventories: &mut [NodeInventory], enrolled_at: &str) {
    for node in inventories {
        node.enrolled_at = Some(enrolled_at.to_string());
    }
}

async fn upload_inventory(
    sink: &InventoryBackendSink,
    inventories: &[NodeInventory],
) -> Result<BackendBatchOutcome, UploadError> {
    sink.write_batch(inventories).await.map_err(|err| {
        if is_auth_failure(err.as_ref()) {
            UploadError::AuthFailure(err.to_string())
        } else {
            UploadError::Other(err.to_string())
        }
    })
}

async fn receive_inventory_batch(
    receiver: &mut tokio::sync::mpsc::Receiver<NodeInventory>,
    max_size: usize,
    max_wait: Duration,
) -> Vec<NodeInventory> {
    let Some(first) = receiver.recv().await else {
        return Vec::new();
    };
    let mut batch = Vec::with_capacity(max_size.max(1));
    batch.push(first);
    let deadline = tokio::time::Instant::now() + max_wait;

    while batch.len() < max_size.max(1) {
        tokio::select! {
            item = receiver.recv() => match item {
                Some(inventory) => batch.push(inventory),
                None => break,
            },
            () = tokio::time::sleep_until(deadline) => break,
        }
    }
    batch
}

async fn upload_inventory_batch(
    sink: &mut InventoryBackendSink,
    session: &mut BackendSession,
    mut inventories: Vec<NodeInventory>,
) -> Result<(Vec<NodeInventory>, Result<BackendBatchOutcome, UploadError>), Box<dyn Error>> {
    stamp_enrolled_at(&mut inventories, session.enrolled_at());
    let mut result = upload_inventory(sink, &inventories).await;
    if let Err(UploadError::AuthFailure(message)) = &result {
        tracing::warn!(
            error = %message,
            "backend JWT rejected; enrolling again before retry"
        );
        session.reenroll().await?;
        sink.set_jwt(session.jwt());
        stamp_enrolled_at(&mut inventories, session.enrolled_at());
        result = upload_inventory(sink, &inventories).await;
    }

    Ok((inventories, result))
}

#[derive(Debug, Eq, PartialEq)]
struct ResolvedNode {
    mac: String,
    node_uuid: String,
}

fn partition_uploaded_inventory<'a>(
    inventories: Vec<NodeInventory>,
    outcomes: impl IntoIterator<Item = BackendNodeOutcome<'a>>,
) -> (Vec<ResolvedNode>, Vec<InventoryUploadFailure>) {
    let mut resolved = Vec::new();
    let mut failures = Vec::new();
    let mut outcomes = outcomes.into_iter();
    for inventory in inventories {
        let mac = inventory.source.mac;
        match outcomes.next().unwrap_or(BackendNodeOutcome::Missing) {
            BackendNodeOutcome::Succeeded { node_uuid } => {
                if let Some(node) = resolved_node(mac, node_uuid) {
                    resolved.push(node);
                }
            }
            BackendNodeOutcome::Rejected { status, message } => {
                failures.push(InventoryUploadFailure::Rejected {
                    mac,
                    status: status.to_string(),
                    message: message.map(str::to_string),
                });
            }
            BackendNodeOutcome::Missing => {
                failures.push(InventoryUploadFailure::MissingResult { mac });
            }
        }
    }
    (resolved, failures)
}

fn resolved_node(mac: String, node_uuid: Option<&str>) -> Option<ResolvedNode> {
    let node_uuid = InventoryUuid::parse(node_uuid?)?.to_string();
    Some(ResolvedNode { mac, node_uuid })
}

#[derive(Debug, Eq, PartialEq)]
enum InventoryUploadFailure {
    Rejected {
        mac: String,
        status: String,
        message: Option<String>,
    },
    MissingResult {
        mac: String,
    },
}

#[derive(Debug)]
enum UploadError {
    AuthFailure(String),
    Other(String),
}

impl fmt::Display for UploadError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::AuthFailure(message) | Self::Other(message) => f.write_str(message),
        }
    }
}

impl Error for UploadError {}

async fn collect_inventory_targets(
    options: &InventoryOptions,
    targets: Vec<config::Target>,
    dispatcher: Arc<Dispatcher>,
) -> Result<Vec<NodeInventory>, Box<dyn Error>> {
    let timeout = std::time::Duration::from_secs(options.timeout_seconds);
    let target_concurrency = options.target_concurrency.max(1);
    let source = RedfishSource::new(options.insecure_tls, timeout, dispatcher)?.with_latency_probe(
        LatencyProbe::for_run(options.latency_probe, TcpProbeConfig::default()).await,
    );

    Ok(inventory_collections(
        &source,
        targets,
        target_concurrency,
        CollectionOrder::Configured,
    )
    .collect()
    .await)
}

#[derive(Clone, Copy)]
enum CollectionOrder {
    Configured,
    Completion,
}

fn inventory_collections<'a>(
    source: &'a RedfishSource,
    targets: Vec<config::Target>,
    concurrency: usize,
    order: CollectionOrder,
) -> Pin<Box<dyn Stream<Item = NodeInventory> + Send + 'a>> {
    let collections = stream::iter(targets).map(|target| source.collect(target));
    match order {
        CollectionOrder::Configured => Box::pin(collections.buffered(concurrency)),
        CollectionOrder::Completion => Box::pin(collections.buffer_unordered(concurrency)),
    }
}

fn start_dispatcher(
    options: &InventoryOptions,
    targets: &[config::Target],
) -> Result<Dispatcher, Box<dyn Error>> {
    let target_concurrency = options.target_concurrency.max(1);
    let global_max_in_flight =
        target_concurrency.saturating_mul(options.bmc_max_in_flight as usize);
    Ok(Dispatcher::start(
        DispatcherConfig {
            global_max_in_flight,
            bmc_max_in_flight: options.bmc_max_in_flight,
            bmc_requests_per_second: options.bmc_requests_per_second,
            bmc_burst: options.bmc_burst,
            workflow_queue_capacity: WORKFLOW_QUEUE_CAPACITY,
        },
        targets.iter().map(|target| target.mac.to_string()),
    )?)
}

#[cfg(test)]
mod tests {
    use bmc_attestation::models::{AttestationEvidence, ComponentError, NodeAttestation};
    use bmc_inventory::models::{DomainError, InventorySourceInfo};

    use super::*;

    #[test]
    fn inventory_summary_distinguishes_success_partial_and_failure() {
        let successful = inventory("10.0.0.1");
        let mut partial = inventory("10.0.0.2");
        partial.domain_errors.push(DomainError {
            domain: "firmware".to_string(),
            resource: None,
            message: "not supported".into(),
        });
        let mut failed = inventory("10.0.0.3");
        failed.target_error = Some("connection refused".into());

        assert_eq!(
            summarize_inventory_collection(&[successful, partial, failed]),
            InventoryCollectionSummary {
                total: 3,
                successful: 1,
                partial: 1,
                failed: 1,
            }
        );
    }

    #[test]
    fn attestation_identity_comes_only_from_valid_backend_uuid() {
        let mac = "00:11:22:33:44:55".to_string();
        assert_eq!(resolved_node(mac.clone(), None), None);
        assert_eq!(resolved_node(mac.clone(), Some("not-a-uuid")), None);
        assert_eq!(resolved_node(mac.clone(), Some("")), None);
        assert_eq!(
            resolved_node(mac.clone(), Some("9AEE71E4-6E80-11EE-A8AC-74D4DD550187")),
            Some(ResolvedNode {
                mac,
                node_uuid: "9aee71e4-6e80-11ee-a8ac-74d4dd550187".to_string(),
            })
        );
    }

    #[test]
    fn upload_results_map_to_mac_by_position_and_use_backend_identity() {
        let first = inventory("10.0.0.1");
        let mut failed_collection = inventory("10.0.0.2");
        failed_collection.source.mac = "00:11:22:33:44:66".to_string();
        failed_collection.target_error = Some("unavailable".into());
        let mut rejected = inventory("10.0.0.3");
        rejected.source.mac = "00:11:22:33:44:77".to_string();
        let mut missing = inventory("10.0.0.4");
        missing.source.mac = "00:11:22:33:44:88".to_string();
        let uuid = "9aee71e4-6e80-11ee-a8ac-74d4dd550187";
        let (nodes, failures) = partition_uploaded_inventory(
            vec![first, failed_collection, rejected, missing],
            [
                BackendNodeOutcome::Succeeded {
                    node_uuid: Some(uuid),
                },
                BackendNodeOutcome::Succeeded { node_uuid: None },
                BackendNodeOutcome::Rejected {
                    status: "CONFLICT",
                    message: Some("ambiguous identity"),
                },
            ],
        );
        assert_eq!(
            nodes,
            vec![ResolvedNode {
                mac: "00:11:22:33:44:55".to_string(),
                node_uuid: uuid.to_string(),
            }]
        );
        assert_eq!(
            failures,
            vec![
                InventoryUploadFailure::Rejected {
                    mac: "00:11:22:33:44:77".to_string(),
                    status: "CONFLICT".to_string(),
                    message: Some("ambiguous identity".to_string()),
                },
                InventoryUploadFailure::MissingResult {
                    mac: "00:11:22:33:44:88".to_string()
                },
            ]
        );
    }

    #[test]
    fn attestation_summary_counts_results_evidence_and_component_errors() {
        let successful = NodeAttestation {
            node_uuid: "node-1".to_string(),
            target_name: "target-1".to_string(),
            bmc_address: "10.0.0.1".to_string(),
            success: true,
            error_message: None,
            nonce_refresh_timestamp: None,
            evidences: vec![AttestationEvidence {
                component_id: "GPU_0".to_string(),
                arch: None,
                vbios_version: None,
                nonce: "secret-nonce".to_string(),
                evidence: "evidence".to_string(),
                certificate: "certificate".to_string(),
                version: None,
            }],
            component_errors: vec![],
        };
        let mut failed = NodeAttestation::failed(
            "node-2".to_string(),
            "target-2".to_string(),
            "10.0.0.2".to_string(),
            "unsupported",
        );
        failed.component_errors.push(ComponentError {
            component_id: Some("GPU_0".to_string()),
            message: "unsupported".into(),
        });
        let batch =
            BatchAttestation::new("2026-01-01T00:00:00Z".to_string(), vec![successful, failed]);

        assert_eq!(
            summarize_attestation_collection(&batch),
            AttestationCollectionSummary {
                total: 2,
                successful: 1,
                failed: 1,
                evidences: 1,
                component_errors: 1,
            }
        );
    }

    #[test]
    fn attestation_batches_follow_target_concurrency() {
        let results = (0..5)
            .map(|index| {
                NodeAttestation::failed(
                    format!("node-{index}"),
                    format!("target-{index}"),
                    format!("10.0.0.{index}"),
                    "unsupported",
                )
            })
            .collect();
        let batch = BatchAttestation::new("2026-01-01T00:00:00Z".to_string(), results);

        let batches = split_attestation_batch(&batch, 2);

        assert_eq!(
            batches
                .iter()
                .map(|batch| batch.results_by_node_uuid.len())
                .collect::<Vec<_>>(),
            [2, 2, 1]
        );
        assert!(batches.iter().all(|batch| {
            batch.schema_version == bmc_attestation::models::ATTESTATION_SCHEMA_VERSION
                && batch.collected_at == "2026-01-01T00:00:00Z"
        }));
    }

    #[test]
    fn build_attestation_targets_surfaces_config_errors() {
        let options = InventoryOptions {
            config: "/definitely/missing/oob-config.toml".to_string(),
            insecure_tls: false,
            detail: false,
            format: InventoryFormat::Json,
            target_concurrency: 1,
            bmc_max_in_flight: 1,
            bmc_requests_per_second: 1,
            bmc_burst: 1,
            timeout_seconds: 1,
            latency_probe: false,
        };

        assert!(build_attestation_targets(&options, &[]).is_err());
    }

    #[test]
    fn runtime_config_uses_effective_options() {
        let runtime = RuntimeOptions {
            inventory: InventoryOptions {
                config: "unused-targets.toml".to_string(),
                insecure_tls: false,
                detail: false,
                format: InventoryFormat::Json,
                target_concurrency: 0,
                bmc_max_in_flight: 1,
                bmc_requests_per_second: 1,
                bmc_burst: 1,
                timeout_seconds: 1,
                latency_probe: false,
            },
            backend_url: Some("https://private.example.com".to_string()),
            agent_token: Some("must-not-be-reported".to_string()),
            interval_seconds: 0,
        };
        let config = runtime.runtime_config();
        assert_eq!(config.inventory_interval_seconds, 1);
        assert_eq!(config.target_concurrency, 1);
        assert_eq!(config.bmc_max_in_flight, 1);
        assert!(!config.latency_probe);
    }

    #[test]
    fn optional_enablement_distinguishes_unknown_false_and_invalid_values() {
        for name in ["FLEETINT_NICO_ENABLED", "FLEETINT_OTEL_GATEWAY_ENABLED"] {
            assert_eq!(
                parse_optional_bool(name, Err(std::env::VarError::NotPresent)).unwrap(),
                None
            );
            assert_eq!(
                parse_optional_bool(name, Ok("true".into())).unwrap(),
                Some(true)
            );
            assert_eq!(
                parse_optional_bool(name, Ok("false".into())).unwrap(),
                Some(false)
            );
            for value in ["", "1", "TRUE", "invalid"] {
                assert_eq!(
                    parse_optional_bool(name, Ok(value.into()))
                        .unwrap_err()
                        .to_string(),
                    format!("{name} must be true or false")
                );
            }
        }
    }

    #[tokio::test]
    async fn inventory_batch_flushes_at_maximum_size() {
        let (tx, mut rx) = tokio::sync::mpsc::channel(3);
        tx.send(inventory("10.0.0.1")).await.expect("send");
        tx.send(inventory("10.0.0.2")).await.expect("send");
        tx.send(inventory("10.0.0.3")).await.expect("send");

        let batch = receive_inventory_batch(&mut rx, 2, Duration::from_secs(60)).await;

        assert_eq!(batch.len(), 2);
        assert_eq!(batch[0].source.address, "10.0.0.1");
        assert_eq!(batch[1].source.address, "10.0.0.2");
    }

    #[tokio::test]
    async fn inventory_batch_flushes_after_maximum_wait() {
        let (tx, mut rx) = tokio::sync::mpsc::channel(1);
        tx.send(inventory("10.0.0.1")).await.expect("send");

        let batch = tokio::time::timeout(
            Duration::from_millis(250),
            receive_inventory_batch(&mut rx, 32, Duration::from_millis(10)),
        )
        .await
        .expect("batch flush timeout");

        assert_eq!(batch.len(), 1);
    }

    #[tokio::test]
    async fn inventory_batch_flushes_when_collection_channel_closes() {
        let (tx, mut rx) = tokio::sync::mpsc::channel(2);
        tx.send(inventory("10.0.0.1")).await.expect("send");
        tx.send(inventory("10.0.0.2")).await.expect("send");
        drop(tx);

        let batch = tokio::time::timeout(
            Duration::from_millis(250),
            receive_inventory_batch(&mut rx, 32, Duration::from_secs(60)),
        )
        .await
        .expect("channel-close flush timeout");

        assert_eq!(batch.len(), 2);
    }

    fn inventory(address: &str) -> NodeInventory {
        NodeInventory {
            schema_version: bmc_inventory::models::INVENTORY_SCHEMA_VERSION.to_string(),
            collected_at: "2026-01-01T00:00:00Z".to_string(),
            enrolled_at: None,
            kind: bmc_inventory::models::TargetKind::Unknown,
            name: None,
            compute_zone: None,
            node_group: None,
            source: InventorySourceInfo {
                source_type: "redfish".to_string(),
                mac: "00:11:22:33:44:55".to_string(),
                address: address.to_string(),
                redfish_version: None,
                service_uuid: None,
                vendor: None,
            },
            managers: vec![],
            systems: vec![],
            chassis: vec![],
            firmware: vec![],
            fabrics: Vec::new(),
            domain_errors: vec![],
            target_error: None,
            latency: Vec::new(),
        }
    }
}
