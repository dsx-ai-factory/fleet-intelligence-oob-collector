use std::error::Error;
use std::fmt;
use std::sync::Arc;

use bmc_attestation::config as attestation_config;
use bmc_attestation::mapper::{AttestationMapper, RedfishMapper as AttestationRedfishMapper};
use bmc_attestation::models::{BatchAttestation, NodeAttestation};
use bmc_attestation::sink::{
    BackendClient as AttestationBackendClient,
    BackendSink as AttestationBackendSink, HttpBackendClient as AttestationHttpBackendClient,
};
use bmc_attestation::source::{
    MeasurementRequest, NonceData, RedfishAttestationSource, now_rfc3339,
};
use bmc_inventory::config;
use bmc_inventory::mapper::{InventoryMapper, RedfishMapper};
use bmc_inventory::models::NodeInventory;
use bmc_inventory::sink::{
    BackendHttpError, BackendSink as InventoryBackendSink,
    HttpBackendClient as InventoryHttpBackendClient, InventorySink, JsonSink, TableSink,
};
use bmc_inventory::source::{InventorySource, RedfishSource};
use futures::stream::{self, StreamExt};

pub const DEFAULT_REDFISH_TIMEOUT_SECONDS: u64 = 30;
pub const DEFAULT_TARGET_CONCURRENCY: usize = 64;
pub const DEFAULT_INVENTORY_INTERVAL_SECONDS: u64 = 24 * 60 * 60;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum InventoryFormat {
    Table,
    Json,
}

#[derive(Clone, Debug)]
pub struct InventoryOptions {
    pub config: String,
    pub target: Option<String>,
    pub insecure_tls: bool,
    pub detail: bool,
    pub format: InventoryFormat,
    pub concurrency: usize,
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
    let backend_url = required_backend_option(options.backend_url, "backend URL")?;
    let endpoints = agent_endpoints(&backend_url)?;
    let agent_token = required_backend_option(options.agent_token, "agent token")?;
    // enrolled_at is captured at enrollment and refreshed whenever the agent
    // re-enrolls (e.g. after a JWT rejection).
    let Enrollment {
        mut jwt,
        mut enrolled_at,
    } = enroll_agent(endpoints.enroll_endpoint.clone(), agent_token.clone()).await?;
    let interval = std::time::Duration::from_secs(options.interval_seconds.max(1));

    let attestation_settings = {
        let mut s = attestation_config::defaults::attestation_config();
        s.insecure_tls = options.inventory.insecure_tls;
        s.concurrency = options.inventory.concurrency;
        s.redfish_timeout = std::time::Duration::from_secs(options.inventory.timeout_seconds);
        s
    };
    let nonce_path = attestation_config::defaults::DEFAULT_BACKEND_NONCE_PATH.to_string();
    let attestation_path =
        attestation_config::defaults::DEFAULT_BACKEND_ATTESTATION_PATH.to_string();

    loop {
        let started_at = std::time::Instant::now();

        // Phase 1: Collect and upload inventory
        match collect_and_upload_inventory(
            options.inventory.clone(),
            endpoints.oob_nodes_endpoint.clone(),
            endpoints.enroll_endpoint.clone(),
            agent_token.clone(),
            &mut jwt,
            &mut enrolled_at,
        )
        .await
        {
            Ok(inventories) => {
                tracing::info!(
                    elapsed_seconds = started_at.elapsed().as_secs_f64(),
                    "inventory backend sync completed"
                );

                // Phase 2: Collect and upload attestation for nodes with discovered UUIDs only when inventory succeeds
                let attestation_targets =
                    build_attestation_targets(&options.inventory, &inventories);
                if attestation_targets.is_empty() {
                    tracing::warn!("no nodes with discovered UUID; skipping attestation");
                } else {
                    match collect_and_upload_attestation(
                        &attestation_targets,
                        &attestation_settings,
                        &backend_url,
                        &nonce_path,
                        &attestation_path,
                        &jwt,
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
            Err(err) => tracing::error!(
                elapsed_seconds = started_at.elapsed().as_secs_f64(),
                next_attempt_seconds = interval.as_secs(),
                error = %err,
                "inventory backend sync failed"
            ),
        }

        tracing::info!(next_sync_seconds = interval.as_secs(), "sleeping until next cycle");
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

struct AttestationTarget {
    name: String,
    node_uuid: String,
    address: String,
    username: String,
    password: String,
}

fn build_attestation_targets(
    options: &InventoryOptions,
    inventories: &[NodeInventory],
) -> Vec<AttestationTarget> {
    let targets = match config::load_targets(&options.config) {
        Ok(targets) => targets,
        Err(_) => return Vec::new(),
    };

    targets
        .into_iter()
        .filter_map(|target| {
            let uuid = inventories.iter().find_map(|inv| {
                if inv.source.address == target.address {
                    inv.uuid.as_ref().map(|u| u.to_string())
                } else {
                    None
                }
            })?;
            Some(AttestationTarget {
                name: target.name,
                node_uuid: uuid,
                address: target.address,
                username: target.username,
                password: target.password,
            })
        })
        .collect()
}

async fn collect_and_upload_attestation(
    targets: &[AttestationTarget],
    attestation_settings: &attestation_config::AttestationConfig,
    backend_url: &str,
    nonce_path: &str,
    attestation_path: &str,
    jwt: &str,
) -> Result<(), Box<dyn Error>> {
    let source = RedfishAttestationSource::new(
        attestation_settings.port,
        attestation_settings.insecure_tls,
        attestation_settings.redfish_timeout,
        attestation_settings.task_timeout,
        attestation_settings.task_poll_interval,
    )?;
    let mapper = AttestationRedfishMapper;
    let measurement = MeasurementRequest {
        measurement_indices: attestation_settings.measurement_indices.clone(),
    };
    let concurrency = attestation_settings.concurrency.max(1);
    let timeout = attestation_settings.redfish_timeout;

    let collected: Vec<_> = stream::iter(targets)
        .map(|target| {
            let source = source.clone();
            let measurement = measurement.clone();
            let backend_url = backend_url.to_string();
            let nonce_path = nonce_path.to_string();
            let attestation_path = attestation_path.to_string();
            let jwt = jwt.to_string();
            let node_uuid = target.node_uuid.clone();
            let config_target = attestation_config::Target {
                name: target.name.clone(),
                node_uuid: target.node_uuid.clone(),
                address: target.address.clone(),
                username: target.username.clone(),
                password: target.password.clone(),
            };
            async move {
                let mut client = match AttestationHttpBackendClient::new(
                    &backend_url,
                    &nonce_path,
                    &attestation_path,
                    jwt,
                    timeout,
                ) {
                    Ok(client) => client,
                    Err(err) => {
                        return NodeAttestation::failed(
                            node_uuid,
                            config_target.name,
                            config_target.address,
                            format!("backend client: {err}"),
                        );
                    }
                };
                let nonce = match client.get_nonce(&node_uuid).await {
                    Ok(response) => response,
                    Err(err) => {
                        return NodeAttestation::failed(
                            node_uuid,
                            config_target.name,
                            config_target.address,
                            format!("nonce request: {err}"),
                        );
                    }
                };
                let record = source
                    .collect(
                        config_target,
                        NonceData {
                            nonce: nonce.nonce,
                            nonce_refresh_timestamp: nonce.nonce_refresh_timestamp,
                        },
                        measurement,
                    )
                    .await;
                mapper.map(record)
            }
        })
        .buffered(concurrency)
        .collect()
        .await;

    let batch = BatchAttestation::new(now_rfc3339(), collected);
    let sink = AttestationBackendSink::new(
        backend_url,
        nonce_path,
        attestation_path,
        jwt,
        timeout,
    )?;
    sink.write(&batch).await?;
    Ok(())
}

pub async fn run_inventory(options: InventoryOptions) -> Result<(), Box<dyn Error>> {
    let inventories = collect_inventory(options.clone()).await?;

    match options.format {
        InventoryFormat::Table => TableSink::new(options.detail).write(&inventories)?,
        InventoryFormat::Json => JsonSink.write(&inventories)?,
    }
    Ok(())
}

async fn collect_and_upload_inventory(
    options: InventoryOptions,
    endpoint: String,
    enroll_endpoint: String,
    agent_token: String,
    jwt: &mut String,
    enrolled_at: &mut String,
) -> Result<Arc<Vec<NodeInventory>>, Box<dyn Error>> {
    let mut inventory_vec = collect_inventory(options).await?;
    stamp_enrolled_at(&mut inventory_vec, enrolled_at);
    let inventories = Arc::new(inventory_vec);
    match upload_inventory(endpoint.clone(), jwt.clone(), Arc::clone(&inventories)).await {
        Ok(()) => Ok(inventories),
        Err(UploadError::AuthFailure(message)) => {
            tracing::info!(
                error = %message,
                "backend JWT rejected; enrolling again before retry"
            );
            // Re-enroll refreshes both the JWT and enrolled_at; re-stamp the nodes
            // so the retry carries the updated enrollment timestamp.
            let enrollment: Enrollment = enroll_agent(enroll_endpoint, agent_token).await?;
            *jwt = enrollment.jwt;
            *enrolled_at = enrollment.enrolled_at;
            let mut inventory_vec = Arc::try_unwrap(inventories)
                .expect("upload task released its inventory reference");
            stamp_enrolled_at(&mut inventory_vec, enrolled_at);
            let inventories = Arc::new(inventory_vec);
            upload_inventory(endpoint, jwt.clone(), Arc::clone(&inventories))
                .await
                .map_err(|err| -> Box<dyn Error> { err.to_string().into() })?;
            Ok(inventories)
        }
        Err(err) => Err(err.to_string().into()),
    }
}

fn stamp_enrolled_at(inventories: &mut [NodeInventory], enrolled_at: &str) {
    for node in inventories {
        node.enrolled_at = Some(enrolled_at.to_string());
    }
}

struct Enrollment {
    jwt: String,
    enrolled_at: String,
}

async fn enroll_agent(
    enroll_endpoint: String,
    agent_token: String,
) -> Result<Enrollment, Box<dyn Error>> {
    let jwt = tokio::task::spawn_blocking(move || {
        InventoryHttpBackendClient::enroll(&enroll_endpoint, &agent_token)
            .map_err(|err| err.to_string())
    })
    .await?
    .map_err(|err| -> Box<dyn Error> { err.into() })?;
    Ok(Enrollment {
        jwt,
        enrolled_at: now_rfc3339(),
    })
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
                if backend_error_is_auth_failure(err.as_ref()) {
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

fn backend_error_is_auth_failure(err: &(dyn Error + 'static)) -> bool {
    err.downcast_ref::<BackendHttpError>()
        .map(|err| matches!(err.status(), 401 | 403))
        .unwrap_or(false)
}

async fn collect_inventory(
    options: InventoryOptions,
) -> Result<Vec<NodeInventory>, Box<dyn Error>> {
    let mut targets = config::load_targets(&options.config)?;
    if let Some(target_name) = &options.target {
        targets.retain(|target| target.name == *target_name);
        if targets.is_empty() {
            return Err(format!("target {target_name:?} not found").into());
        }
    }

    let timeout = std::time::Duration::from_secs(options.timeout_seconds);
    let source = RedfishSource::new(options.insecure_tls, timeout)?;
    let mapper = RedfishMapper;
    let concurrency = options.concurrency.max(1);

    Ok(stream::iter(targets)
        .map(|target| async {
            let record = source.collect(target).await;
            mapper.map(record)
        })
        .buffered(concurrency)
        .collect()
        .await)
}

fn required_backend_option(
    value: Option<String>,
    name: &'static str,
) -> Result<String, Box<dyn Error>> {
    value
        .filter(|value| !value.trim().is_empty())
        .ok_or_else(|| format!("{name} is required when running backend runtime").into())
}

#[derive(Clone, Debug, Eq, PartialEq)]
struct AgentEndpoints {
    enroll_endpoint: String,
    oob_nodes_endpoint: String,
}

fn agent_endpoints(backend_url: &str) -> Result<AgentEndpoints, Box<dyn Error>> {
    let backend_url = backend_url.trim().trim_end_matches('/');
    if backend_url.ends_with("/v1/agent/enroll") || backend_url.ends_with("/v1/agent/oob_nodes") {
        return Err("backend URL must be the API base URL without a /v1/agent path suffix".into());
    }

    Ok(AgentEndpoints {
        enroll_endpoint: format!("{backend_url}/v1/agent/enroll"),
        oob_nodes_endpoint: format!("{backend_url}/v1/agent/oob_nodes"),
    })
}

#[cfg(test)]
mod tests {
    use super::{AgentEndpoints, agent_endpoints, backend_error_is_auth_failure};
    use bmc_inventory::sink::BackendHttpError;
    use std::error::Error;

    #[test]
    fn agent_endpoints_accepts_base_url() {
        assert_eq!(
            agent_endpoints("https://api.example.com/").expect("endpoints"),
            AgentEndpoints {
                enroll_endpoint: "https://api.example.com/v1/agent/enroll".to_string(),
                oob_nodes_endpoint: "https://api.example.com/v1/agent/oob_nodes".to_string(),
            }
        );
    }

    #[test]
    fn agent_endpoints_trims_trailing_slashes() {
        assert_eq!(
            agent_endpoints("https://api.example.com///").expect("endpoints"),
            AgentEndpoints {
                enroll_endpoint: "https://api.example.com/v1/agent/enroll".to_string(),
                oob_nodes_endpoint: "https://api.example.com/v1/agent/oob_nodes".to_string(),
            }
        );
    }

    #[test]
    fn agent_endpoints_rejects_full_endpoint_urls() {
        let err = agent_endpoints("https://api.example.com/v1/agent/enroll").expect_err("error");
        assert!(err.to_string().contains("API base URL"));

        let err = agent_endpoints("https://api.example.com/v1/agent/oob_nodes").expect_err("error");
        assert!(err.to_string().contains("API base URL"));
    }

    #[test]
    fn backend_auth_failure_detects_jwt_rejection_statuses() {
        let unauthorized: Box<dyn Error> = Box::new(BackendHttpError::new(401, "expired token"));
        let forbidden: Box<dyn Error> = Box::new(BackendHttpError::new(403, "invalid token"));
        let server_error: Box<dyn Error> = Box::new(BackendHttpError::new(500, "backend failed"));

        assert!(backend_error_is_auth_failure(unauthorized.as_ref()));
        assert!(backend_error_is_auth_failure(forbidden.as_ref()));
        assert!(!backend_error_is_auth_failure(server_error.as_ref()));
    }
}
