use std::error::Error;
use std::fmt;
use std::sync::Arc;

use bmc_attestation::config as attestation_config;
use bmc_attestation::mapper::{AttestationMapper, RedfishMapper as AttestationRedfishMapper};
use bmc_attestation::models::BatchAttestation;
use bmc_attestation::sink::{
    AttestationSink, BackendClient as AttestationBackendClient,
    BackendSink as AttestationBackendSink, HttpBackendClient as AttestationHttpBackendClient,
    JsonSink as AttestationJsonSink, TableSink as AttestationTableSink,
};
use bmc_attestation::source::{
    MeasurementRequest, NonceData, RedfishAttestationSource, SourceRecord, now_rfc3339,
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
use tokio::time::{sleep, timeout};

pub const DEFAULT_REDFISH_TIMEOUT_SECONDS: u64 = 30;
const LOCAL_TEST_NONCE: &str = "000102030405060708090a0b0c0d0e0f101112131415161718191a1b1c1d1e1f";
pub const DEFAULT_TARGET_CONCURRENCY: usize = 64;
pub const DEFAULT_INVENTORY_INTERVAL_SECONDS: u64 = 24 * 60 * 60;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum InventoryFormat {
    Table,
    Json,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum AttestationFormat {
    Table,
    Json,
    Backend,
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

pub struct AttestationOptions {
    pub config: String,
    pub target: Option<String>,
    pub detail: bool,
    pub format: AttestationFormat,
}

#[derive(Clone, Debug)]
pub struct RuntimeOptions {
    pub inventory: InventoryOptions,
    pub backend_url: Option<String>,
    pub agent_token: Option<String>,
    pub interval_seconds: u64,
}

pub async fn run_runtime(options: RuntimeOptions) -> Result<(), Box<dyn Error>> {
    let endpoints = agent_endpoints(&required_backend_option(
        options.backend_url,
        "backend URL",
    )?)?;
    let agent_token = required_backend_option(options.agent_token, "agent token")?;
    let mut jwt = enroll_agent(endpoints.enroll_endpoint.clone(), agent_token.clone()).await?;

    let interval = std::time::Duration::from_secs(options.interval_seconds.max(1));
    loop {
        let started_at = std::time::Instant::now();
        match collect_and_upload_inventory(
            options.inventory.clone(),
            endpoints.oob_nodes_endpoint.clone(),
            endpoints.enroll_endpoint.clone(),
            agent_token.clone(),
            &mut jwt,
        )
        .await
        {
            Ok(()) => tracing::info!(
                elapsed_seconds = started_at.elapsed().as_secs_f64(),
                next_sync_seconds = interval.as_secs(),
                "inventory backend sync completed"
            ),
            Err(err) => tracing::error!(
                elapsed_seconds = started_at.elapsed().as_secs_f64(),
                next_attempt_seconds = interval.as_secs(),
                error = %err,
                "inventory backend sync failed"
            ),
        }

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
) -> Result<(), Box<dyn Error>> {
    let inventories = Arc::new(collect_inventory(options).await?);
    match upload_inventory(endpoint.clone(), jwt.clone(), Arc::clone(&inventories)).await {
        Ok(()) => Ok(()),
        Err(UploadError::AuthFailure(message)) => {
            tracing::info!(
                error = %message,
                "backend JWT rejected; enrolling again before retry"
            );
            *jwt = enroll_agent(enroll_endpoint, agent_token).await?;
            upload_inventory(endpoint, jwt.clone(), inventories)
                .await
                .map_err(|err| -> Box<dyn Error> { err.to_string().into() })
        }
        Err(err) => Err(err.to_string().into()),
    }
}

async fn enroll_agent(
    enroll_endpoint: String,
    agent_token: String,
) -> Result<String, Box<dyn Error>> {
    tokio::task::spawn_blocking(move || {
        InventoryHttpBackendClient::enroll(&enroll_endpoint, &agent_token)
            .map_err(|err| err.to_string())
    })
    .await?
    .map_err(|err| -> Box<dyn Error> { err.into() })
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

pub async fn run_attestation(options: AttestationOptions) -> Result<(), Box<dyn Error>> {
    let mut config = attestation_config::load_config(&options.config)?;
    if let Some(target_name) = &options.target {
        config.targets.retain(|target| target.name == *target_name);
        if config.targets.is_empty() {
            return Err(format!("target {target_name:?} not found").into());
        }
    }

    loop {
        let attempt = async {
            let source = RedfishAttestationSource::new(
                config.attestation.port,
                config.attestation.insecure_tls,
                config.attestation.redfish_timeout,
                config.attestation.task_timeout,
                config.attestation.task_poll_interval,
            )?;
            let mapper = AttestationRedfishMapper;
            let measurement = MeasurementRequest {
                measurement_indices: config.attestation.measurement_indices.clone(),
            };
            let concurrency = config.attestation.concurrency.max(1);
            let use_local_nonce = options.format != AttestationFormat::Backend;

            let collected: Vec<_> = stream::iter(config.targets.clone())
                .map(|target| {
                    let source = source.clone();
                    let backend = config.backend.clone();
                    let measurement = measurement.clone();
                    let timeout = config.attestation.redfish_timeout;
                    async move {
                        if use_local_nonce {
                            let record = source
                                .collect(
                                    target,
                                    NonceData {
                                        nonce: LOCAL_TEST_NONCE.to_string(),
                                        nonce_refresh_timestamp: None,
                                    },
                                    measurement,
                                )
                                .await;
                            return (mapper.map(record), None);
                        }

                        let original_jwt = backend.jwt.clone();
                        let mut client = match AttestationHttpBackendClient::new(
                            &backend.url,
                            &backend.nonce_path,
                            &backend.attestation_path,
                            backend.jwt,
                            timeout,
                        ) {
                            Ok(client) => client,
                            Err(err) => {
                                return (
                                    mapper.map(SourceRecord::failed(
                                        target,
                                        None,
                                        format!("backend client: {err}"),
                                    )),
                                    None,
                                );
                            }
                        };
                        let nonce = match client.get_nonce(&target.node_uuid).await {
                            Ok(response) => response,
                            Err(err) => {
                                return (
                                    mapper.map(SourceRecord::failed(
                                        target,
                                        None,
                                        format!("nonce request: {err}"),
                                    )),
                                    None,
                                );
                            }
                        };
                        let refreshed_jwt = if client.jwt() != original_jwt {
                            Some(client.jwt().to_string())
                        } else {
                            None
                        };
                        let record = source
                            .collect(
                                target,
                                NonceData {
                                    nonce: nonce.nonce,
                                    nonce_refresh_timestamp: nonce.nonce_refresh_timestamp,
                                },
                                measurement,
                            )
                            .await;
                        (mapper.map(record), refreshed_jwt)
                    }
                })
                .buffered(concurrency)
                .collect()
                .await;

            let mut refreshed_jwt = None;
            let attestations = collected
                .into_iter()
                .map(|(attestation, jwt)| {
                    if refreshed_jwt.is_none() {
                        refreshed_jwt = jwt;
                    }
                    attestation
                })
                .collect();
            let batch = BatchAttestation::new(now_rfc3339(), attestations);

            match options.format {
                AttestationFormat::Table => {
                    AttestationTableSink::new(options.detail).write(&batch)?
                }
                AttestationFormat::Json => AttestationJsonSink.write(&batch)?,
                AttestationFormat::Backend => {
                    let sink = AttestationBackendSink::new(
                        &config.backend.url,
                        &config.backend.nonce_path,
                        &config.backend.attestation_path,
                        refreshed_jwt.as_deref().unwrap_or(&config.backend.jwt),
                        config.attestation.redfish_timeout,
                    )?;
                    sink.write(&batch).await?;
                }
            }

            Ok(())
        };

        let result = match config.attestation.batch_timeout {
            Some(batch_timeout) => timeout(batch_timeout, attempt).await.map_err(|err| {
                format!("attestation batch timed out after {batch_timeout:?}: {err}")
            })?,
            None => attempt.await,
        };

        match (&result, config.attestation.interval) {
            (Err(_), Some(_)) => sleep(config.attestation.retry_interval).await,
            (Ok(_), Some(interval)) => sleep(interval).await,
            (_, None) => return result,
        }
    }
}
