use std::error::Error;
use std::fmt;
use std::sync::Arc;

use bmc_inventory::config;
use bmc_inventory::mapper::{InventoryMapper, RedfishMapper};
use bmc_inventory::models::NodeInventory;
use bmc_inventory::sink::{
    BackendHttpError, BackendSink, HttpBackendClient, InventorySink, JsonSink, TableSink,
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
        HttpBackendClient::enroll(&enroll_endpoint, &agent_token).map_err(|err| err.to_string())
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
        BackendSink::new(endpoint, jwt)
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
