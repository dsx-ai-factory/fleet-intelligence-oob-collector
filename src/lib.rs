mod backend;

use std::error::Error;
use std::fmt;
use std::sync::Arc;
use std::time::Duration;

use bmc_attestation::collector;
use bmc_attestation::config as attestation_config;
use bmc_attestation::models::BatchAttestation;
use bmc_attestation::sink::BackendSink as AttestationBackendSink;
use bmc_dispatch::{Dispatcher, DispatcherConfig};
use bmc_inventory::config;
use bmc_inventory::mapper::{InventoryMapper, RedfishMapper};
use bmc_inventory::models::NodeInventory;
use bmc_inventory::sink::{
    BackendBatchOutcome, BackendNodeOutcome, BackendSink as InventoryBackendSink, InventorySink,
    JsonSink, TableSink,
};
use bmc_inventory::source::{InventorySource, RedfishSource};
use futures::stream::{self, StreamExt};

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
}

#[derive(Clone, Debug)]
pub struct RuntimeOptions {
    pub inventory: InventoryOptions,
    pub backend_url: Option<String>,
    pub agent_token: Option<String>,
    pub interval_seconds: u64,
}

pub async fn run_runtime(options: RuntimeOptions) -> Result<(), Box<dyn Error>> {
    let configured_targets = config::load_targets(&options.inventory.config)?;
    let dispatcher = Arc::new(start_dispatcher(&options.inventory, &configured_targets)?);
    let backend_url = required_backend_option(options.backend_url, "backend URL")?;
    let endpoints = BackendEndpoints::resolve(&backend_url)?;
    let agent_token = required_backend_option(options.agent_token, "agent token")?;
    // The session owns the JWT lifecycle: enrolled_at is captured at enrollment and
    // refreshed whenever the agent re-enrolls (e.g. after a JWT rejection).
    let mut session =
        BackendSession::enroll(endpoints.enroll_endpoint.clone(), agent_token).await?;
    let interval = std::time::Duration::from_secs(options.interval_seconds.max(1));

    let attestation_settings = {
        let mut s = attestation_config::defaults::attestation_config();
        s.insecure_tls = options.inventory.insecure_tls;
        s.concurrency = options.inventory.target_concurrency;
        s.redfish_timeout = std::time::Duration::from_secs(options.inventory.timeout_seconds);
        s
    };

    loop {
        let started_at = std::time::Instant::now();

        // Phase 1: Collect and upload inventory
        match collect_and_upload_inventory(
            options.inventory.clone(),
            endpoints.oob_nodes_endpoint.clone(),
            &mut session,
            Arc::clone(&dispatcher),
        )
        .await
        {
            Ok(outcome) => {
                if outcome.failed_uploads > 0 {
                    tracing::warn!(
                        elapsed_seconds = started_at.elapsed().as_secs_f64(),
                        exported_count = outcome.inventories.len(),
                        failed_upload_count = outcome.failed_uploads,
                        "inventory backend sync completed with failed node exports"
                    );
                } else {
                    tracing::info!(
                        elapsed_seconds = started_at.elapsed().as_secs_f64(),
                        exported_count = outcome.inventories.len(),
                        "inventory backend sync completed"
                    );
                }
                if outcome.unexportable > 0 {
                    tracing::warn!(
                        elapsed_seconds = started_at.elapsed().as_secs_f64(),
                        unexportable_count = outcome.unexportable,
                        "inventory records without UUIDs were not exported"
                    );
                }

                // Phase 2: Attest only nodes whose inventory export succeeded and
                // produced a UUID known to the backend.
                match build_attestation_targets(&options.inventory, &outcome.inventories) {
                    Err(err) => tracing::error!(
                        error = %err,
                        "failed to load targets for attestation; skipping attestation"
                    ),
                    Ok(attestation_targets) if attestation_targets.is_empty() => {
                        tracing::warn!("no nodes with discovered UUID; skipping attestation");
                    }
                    Ok(attestation_targets) => {
                        match collect_and_upload_attestation(
                            &attestation_targets,
                            &attestation_settings,
                            &endpoints,
                            &mut session,
                            dispatcher.as_ref(),
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
            signal = tokio::signal::ctrl_c() => {
                if let Err(err) = signal {
                    return Err(format!("failed to listen for shutdown signal: {err}").into());
                }
                tracing::info!("shutdown signal received; stopping OOB runtime");
                return Ok(());
            }
        }
    }
}

fn build_attestation_targets(
    options: &InventoryOptions,
    inventories: &[NodeInventory],
) -> Result<Vec<attestation_config::Target>, Box<dyn Error>> {
    let targets = config::load_targets(&options.config)?;

    Ok(targets
        .into_iter()
        .filter_map(|target| {
            let uuid = inventories.iter().find_map(|inv| {
                if inv.source.mac == target.mac {
                    inv.uuid.as_ref().map(|u| u.to_string())
                } else {
                    None
                }
            })?;
            Some(attestation_config::Target {
                name: target.mac,
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
) -> Result<(), Box<dyn Error>> {
    let outcome = match collect_attestation(
        targets,
        attestation_settings,
        endpoints,
        session.jwt(),
        dispatcher,
    )
    .await
    {
        Ok(outcome) => outcome,
        // A rejected JWT during nonce collection cannot be retried per target; re-enroll
        // and re-run the whole batch with a fresh token before giving up.
        Err(err) if is_auth_failure(err.as_ref()) => {
            tracing::warn!(
                error = %err,
                "backend JWT rejected during nonce collection; enrolling again before retry"
            );
            session.reenroll().await?;
            collect_attestation(
                targets,
                attestation_settings,
                endpoints,
                session.jwt(),
                dispatcher,
            )
            .await?
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
    )
    .await
}

async fn upload_attestation_batches(
    endpoints: &BackendEndpoints,
    session: &mut BackendSession,
    batch: &BatchAttestation,
    timeout: Duration,
    batch_size: usize,
) -> Result<(), Box<dyn Error>> {
    let batches = split_attestation_batch(batch, batch_size);
    let batch_count = batches.len();
    let mut failures = Vec::new();

    for (index, batch) in batches.into_iter().enumerate() {
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
    endpoint: String,
    session: &mut BackendSession,
    dispatcher: Arc<Dispatcher>,
) -> Result<InventorySyncOutcome, Box<dyn Error>> {
    let collector_config = config::load_config(&options.config)?;
    dispatcher.sync_endpoints(
        collector_config
            .targets
            .iter()
            .map(|target| target.mac.clone()),
    )?;
    let machine_labels: bmc_inventory::sink::MachineLabels = collector_config
        .targets
        .iter()
        .map(|target| (target.mac.to_ascii_lowercase(), target.labels.clone()))
        .collect();
    let mut backend_sink = InventoryBackendSink::new(endpoint, session.jwt(), machine_labels)?;
    let timeout = Duration::from_secs(options.timeout_seconds);
    let target_concurrency = options.target_concurrency.max(1);
    let source = RedfishSource::new(options.insecure_tls, timeout, dispatcher)?;
    let mapper = RedfishMapper::new();
    let channel_capacity = target_concurrency.saturating_mul(2).max(1);
    let (tx, mut rx) = tokio::sync::mpsc::channel(channel_capacity);
    let producer = tokio::spawn(async move {
        let mut collections = stream::iter(collector_config.targets)
            .map(|target| async {
                let record = source.collect(target).await;
                mapper.map(record).await
            })
            .buffer_unordered(target_concurrency);
        while let Some(inventory) = collections.next().await {
            if tx.send(inventory).await.is_err() {
                return Err("inventory export worker stopped before collection completed");
            }
        }
        Ok::<(), &'static str>(())
    });

    let mut inventory_vec = Vec::new();
    let mut summary = InventoryCollectionSummary::default();
    let mut failed_uploads = 0;
    let mut unexportable = 0;
    loop {
        let batch =
            receive_inventory_batch(&mut rx, target_concurrency, INVENTORY_EXPORT_FLUSH_INTERVAL)
                .await;
        if batch.is_empty() {
            break;
        }
        summary.add(summarize_inventory_collection(&batch));
        let batch_size = batch.len();
        let (exportable, batch_unexportable) = partition_exportable_inventory(batch);
        unexportable += batch_unexportable;
        if batch_unexportable > 0 {
            tracing::warn!(
                batch_size,
                unexportable_count = batch_unexportable,
                "inventory mini-batch contains records without UUIDs; skipping export"
            );
        }
        if exportable.is_empty() {
            continue;
        }
        let exportable_count = exportable.len();
        let (uploaded, upload_result) =
            upload_inventory_batch(&mut backend_sink, session, exportable).await?;
        match upload_result {
            Ok(outcome) => {
                let (accepted, failures) = partition_uploaded_inventory(uploaded, &outcome);
                failed_uploads += failures.len();
                for failure in &failures {
                    match failure {
                        InventoryUploadFailure::Rejected {
                            node_uuid,
                            status,
                            message,
                        } => tracing::warn!(
                            node_uuid,
                            status,
                            message = message.as_deref().unwrap_or(""),
                            "backend rejected inventory node"
                        ),
                        InventoryUploadFailure::MissingResult { node_uuid } => tracing::warn!(
                            node_uuid,
                            "backend returned success without a result row for inventory node"
                        ),
                    }
                }
                if !failures.is_empty() {
                    tracing::warn!(
                        exportable_count,
                        accepted_count = accepted.len(),
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
        .await
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
        inventories: Arc::new(inventory_vec),
        failed_uploads,
        unexportable,
    })
}

struct InventorySyncOutcome {
    inventories: Arc<Vec<NodeInventory>>,
    failed_uploads: usize,
    unexportable: usize,
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
    sink: InventoryBackendSink,
    inventories: Arc<Vec<NodeInventory>>,
) -> Result<BackendBatchOutcome, UploadError> {
    tokio::task::spawn_blocking(move || {
        sink.write_batch(inventories.as_slice()).map_err(|err| {
            if is_auth_failure(err.as_ref()) {
                UploadError::AuthFailure(err.to_string())
            } else {
                UploadError::Other(err.to_string())
            }
        })
    })
    .await
    .map_err(|err| UploadError::Other(err.to_string()))?
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
    let inventories = Arc::new(inventories);
    let mut result = upload_inventory(sink.clone(), Arc::clone(&inventories)).await;

    let mut inventories =
        Arc::try_unwrap(inventories).expect("upload task released its inventory reference");
    if let Err(UploadError::AuthFailure(message)) = &result {
        tracing::warn!(
            error = %message,
            "backend JWT rejected; enrolling again before retry"
        );
        session.reenroll().await?;
        sink.set_jwt(session.jwt());
        stamp_enrolled_at(&mut inventories, session.enrolled_at());
        let retry_inventories = Arc::new(inventories);
        result = upload_inventory(sink.clone(), Arc::clone(&retry_inventories)).await;
        inventories = Arc::try_unwrap(retry_inventories)
            .expect("upload task released its inventory reference");
    }

    Ok((inventories, result))
}

fn partition_uploaded_inventory(
    inventories: Vec<NodeInventory>,
    outcome: &BackendBatchOutcome,
) -> (Vec<NodeInventory>, Vec<InventoryUploadFailure>) {
    let mut accepted = Vec::new();
    let mut failures = Vec::new();
    for inventory in inventories {
        let Some(node_uuid) = inventory.uuid.as_ref().map(ToString::to_string) else {
            continue;
        };
        match outcome.node_outcome(&node_uuid) {
            BackendNodeOutcome::Succeeded => accepted.push(inventory),
            BackendNodeOutcome::Rejected { status, message } => {
                failures.push(InventoryUploadFailure::Rejected {
                    node_uuid,
                    status: status.to_string(),
                    message: message.map(str::to_string),
                });
            }
            BackendNodeOutcome::Missing => {
                failures.push(InventoryUploadFailure::MissingResult { node_uuid });
            }
        }
    }
    (accepted, failures)
}

#[derive(Debug, Eq, PartialEq)]
enum InventoryUploadFailure {
    Rejected {
        node_uuid: String,
        status: String,
        message: Option<String>,
    },
    MissingResult {
        node_uuid: String,
    },
}

fn partition_exportable_inventory(inventories: Vec<NodeInventory>) -> (Vec<NodeInventory>, usize) {
    let total = inventories.len();
    let exportable = inventories
        .into_iter()
        .filter(|inventory| inventory.uuid.is_some())
        .collect::<Vec<_>>();
    let unexportable = total.saturating_sub(exportable.len());
    (exportable, unexportable)
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
    let source = RedfishSource::new(options.insecure_tls, timeout, dispatcher)?;
    let mapper = RedfishMapper::new();

    Ok(stream::iter(targets)
        .map(|target| async {
            let record = source.collect(target).await;
            mapper.map(record).await
        })
        .buffered(target_concurrency)
        .collect()
        .await)
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
        targets.iter().map(|target| target.mac.clone()),
    )?)
}

#[cfg(test)]
mod tests {
    use bmc_attestation::models::{AttestationEvidence, ComponentError, NodeAttestation};
    use bmc_inventory::models::{DomainError, InventorySourceInfo, InventoryUuid};

    use super::*;

    #[test]
    fn inventory_summary_distinguishes_success_partial_and_failure() {
        let successful = inventory("10.0.0.1");
        let mut partial = inventory("10.0.0.2");
        partial.domain_errors.push(DomainError {
            domain: "firmware".to_string(),
            resource: None,
            message: "not supported".to_string(),
        });
        let mut failed = inventory("10.0.0.3");
        failed.target_error = Some("connection refused".to_string());

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
    fn inventory_without_uuid_is_unexportable() {
        let mut exportable = inventory("10.0.0.1");
        exportable.uuid = InventoryUuid::parse("9AEE71E4-6E80-11EE-A8AC-74D4DD550187");
        let mut collection_failure = inventory("10.0.0.2");
        collection_failure.target_error = Some("connection refused".to_string());

        let (exportable, unexportable) =
            partition_exportable_inventory(vec![exportable, collection_failure]);

        assert_eq!(exportable.len(), 1);
        assert_eq!(exportable[0].source.address, "10.0.0.1");
        assert_eq!(unexportable, 1);
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
            message: "unsupported".to_string(),
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
        };

        assert!(build_attestation_targets(&options, &[]).is_err());
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
            uuid: None,
            source: InventorySourceInfo {
                source_type: "redfish".to_string(),
                mac: "00:11:22:33:44:55".to_string(),
                address: address.to_string(),
                host_name: None,
                redfish_version: None,
                service_uuid: None,
                vendor: None,
            },
            primary_system_id: None,
            managers: vec![],
            systems: vec![],
            chassis: vec![],
            firmware: vec![],
            domain_errors: vec![],
            target_error: None,
        }
    }
}
