mod backend;

use std::error::Error;
use std::fmt;
use std::sync::Arc;

use bmc_attestation::collector;
use bmc_attestation::config as attestation_config;
use bmc_attestation::models::BatchAttestation;
use bmc_attestation::sink::BackendSink as AttestationBackendSink;
use bmc_inventory::config;
use bmc_inventory::mapper::{InventoryMapper, RedfishMapper};
use bmc_inventory::models::NodeInventory;
use bmc_inventory::sink::{
    BackendSink as InventoryBackendSink, InventorySink, JsonSink, NicoHealthSink, TableSink,
};
use bmc_inventory::source::{InventorySource, RedfishSource};
use futures::stream::{self, StreamExt};

use backend::{BackendEndpoints, BackendSession, is_auth_failure, required_backend_option};

pub const DEFAULT_REDFISH_TIMEOUT_SECONDS: u64 = 30;
pub const DEFAULT_TARGET_CONCURRENCY: usize = 64;
pub const DEFAULT_INVENTORY_INTERVAL_SECONDS: u64 = 24 * 60 * 60;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum InventoryFormat {
    Table,
    Json,
    NicoHealth,
}

#[derive(Clone, Debug)]
pub struct InventoryOptions {
    pub config: String,
    pub insecure_tls: bool,
    pub detail: bool,
    pub format: InventoryFormat,
    pub concurrency: usize,
    pub timeout_seconds: u64,
    pub output: Option<String>,
}

#[derive(Clone, Debug)]
pub struct RuntimeOptions {
    pub inventory: InventoryOptions,
    pub backend_url: Option<String>,
    pub agent_token: Option<String>,
    pub interval_seconds: u64,
}

pub async fn run_runtime(options: RuntimeOptions) -> Result<(), Box<dyn Error>> {
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
        s.concurrency = options.inventory.concurrency;
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
        )
        .await
        {
            Ok(inventories) => {
                tracing::info!(
                    elapsed_seconds = started_at.elapsed().as_secs_f64(),
                    "inventory backend sync completed"
                );

                // Phase 2: Collect and upload attestation for nodes with discovered UUIDs only when inventory succeeds
                match build_attestation_targets(&options.inventory, &inventories) {
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
) -> Result<(), Box<dyn Error>> {
    let outcome =
        match collect_attestation(targets, attestation_settings, endpoints, session.jwt()).await {
            Ok(outcome) => outcome,
            // A rejected JWT during nonce collection cannot be retried per target; re-enroll
            // and re-run the whole batch with a fresh token before giving up.
            Err(err) if is_auth_failure(err.as_ref()) => {
                tracing::warn!(
                    error = %err,
                    "backend JWT rejected during nonce collection; enrolling again before retry"
                );
                session.reenroll().await?;
                collect_attestation(targets, attestation_settings, endpoints, session.jwt()).await?
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

    let timeout = attestation_settings.redfish_timeout;
    match upload_attestation(endpoints, session.jwt(), &outcome.batch, timeout).await {
        Ok(()) => Ok(()),
        Err(err) if is_auth_failure(err.as_ref()) => {
            tracing::warn!(
                error = %err,
                "backend JWT rejected on attestation upload; enrolling again before retry"
            );
            session.reenroll().await?;
            upload_attestation(endpoints, session.jwt(), &outcome.batch, timeout).await
        }
        Err(err) => Err(err),
    }
}

async fn collect_attestation(
    targets: &[attestation_config::Target],
    attestation_settings: &attestation_config::AttestationConfig,
    endpoints: &BackendEndpoints,
    jwt: &str,
) -> Result<collector::CollectionOutcome, Box<dyn Error>> {
    collector::collect(
        targets,
        attestation_settings,
        &endpoints.base_url,
        &endpoints.nonce_path,
        &endpoints.attestation_path,
        jwt,
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
    if options.format == InventoryFormat::NicoHealth && options.output.is_none() {
        return Err("--output is required when --format nico-health is selected".into());
    }
    let inventories = collect_inventory(options.clone()).await?;

    match options.format {
        InventoryFormat::Table => TableSink::new(options.detail).write(&inventories)?,
        InventoryFormat::Json => JsonSink.write(&inventories)?,
        InventoryFormat::NicoHealth => {
            let output = options
                .output
                .as_deref()
                .expect("NICo health output validated before collection");
            let targets = config::load_targets(&options.config)?;
            NicoHealthSink::new(output, targets).write(&inventories)?;
        }
    }
    Ok(())
}

async fn collect_and_upload_inventory(
    options: InventoryOptions,
    endpoint: String,
    session: &mut BackendSession,
) -> Result<Arc<Vec<NodeInventory>>, Box<dyn Error>> {
    let mut inventory_vec = collect_inventory(options).await?;
    let summary = summarize_inventory_collection(&inventory_vec);
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
    stamp_enrolled_at(&mut inventory_vec, session.enrolled_at());
    let inventories = Arc::new(inventory_vec);
    match upload_inventory(
        endpoint.clone(),
        session.jwt().to_string(),
        Arc::clone(&inventories),
    )
    .await
    {
        Ok(()) => Ok(inventories),
        Err(UploadError::AuthFailure(message)) => {
            tracing::warn!(
                error = %message,
                "backend JWT rejected; enrolling again before retry"
            );
            // Re-enroll refreshes both the JWT and enrolled_at; re-stamp the nodes
            // so the retry carries the updated enrollment timestamp.
            session.reenroll().await?;
            let mut inventory_vec =
                Arc::try_unwrap(inventories).expect("upload task released its inventory reference");
            stamp_enrolled_at(&mut inventory_vec, session.enrolled_at());
            let inventories = Arc::new(inventory_vec);
            upload_inventory(
                endpoint,
                session.jwt().to_string(),
                Arc::clone(&inventories),
            )
            .await
            .map_err(|err| -> Box<dyn Error> { err.to_string().into() })?;
            Ok(inventories)
        }
        Err(err) => Err(err.to_string().into()),
    }
}

#[derive(Debug, Eq, PartialEq)]
struct InventoryCollectionSummary {
    total: usize,
    successful: usize,
    partial: usize,
    failed: usize,
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
    endpoint: String,
    jwt: String,
    inventories: Arc<Vec<NodeInventory>>,
) -> Result<(), UploadError> {
    tokio::task::spawn_blocking(move || {
        InventoryBackendSink::new(endpoint, jwt)
            .map_err(|err| UploadError::Other(err.to_string()))?
            .write(inventories.as_slice())
            .map_err(|err| {
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

async fn collect_inventory(
    options: InventoryOptions,
) -> Result<Vec<NodeInventory>, Box<dyn Error>> {
    let targets = config::load_targets(&options.config)?;

    let timeout = std::time::Duration::from_secs(options.timeout_seconds);
    let source = RedfishSource::new(options.insecure_tls, timeout)?;
    let mapper = RedfishMapper::new();
    let concurrency = options.concurrency.max(1);

    Ok(stream::iter(targets)
        .map(|target| async {
            let record = source.collect(target).await;
            mapper.map(record).await
        })
        .buffered(concurrency)
        .collect()
        .await)
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
    fn build_attestation_targets_surfaces_config_errors() {
        let options = InventoryOptions {
            config: "/definitely/missing/oob-config.toml".to_string(),
            insecure_tls: false,
            detail: false,
            format: InventoryFormat::Json,
            concurrency: 1,
            timeout_seconds: 1,
            output: None,
        };

        assert!(build_attestation_targets(&options, &[]).is_err());
    }

    #[tokio::test]
    async fn nico_health_format_requires_output_before_collection() {
        let options = InventoryOptions {
            config: "/definitely/missing/oob-config.toml".to_string(),
            insecure_tls: false,
            detail: false,
            format: InventoryFormat::NicoHealth,
            concurrency: 1,
            timeout_seconds: 1,
            output: None,
        };

        let error = run_inventory(options)
            .await
            .expect_err("missing output should fail")
            .to_string();

        assert!(error.contains("--output is required"));
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
