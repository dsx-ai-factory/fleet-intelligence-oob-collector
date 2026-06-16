use std::error::Error;

use bmc_attestation::config as attestation_config;
use bmc_attestation::mapper::{AttestationMapper, RedfishMapper as AttestationRedfishMapper};
use bmc_attestation::models::BatchAttestation;
use bmc_attestation::sink::{
    AttestationSink, BackendClient, BackendSink, HttpBackendClient,
    JsonSink as AttestationJsonSink, TableSink as AttestationTableSink,
};
use bmc_attestation::source::{
    MeasurementRequest, NonceData, RedfishAttestationSource, SourceRecord, now_rfc3339,
};
use bmc_inventory::config;
use bmc_inventory::mapper::{InventoryMapper, RedfishMapper};
use bmc_inventory::sink::{InventorySink, JsonSink, TableSink};
use bmc_inventory::source::{InventorySource, RedfishSource};
use futures::stream::{self, StreamExt};
use tokio::time::{sleep, timeout};

pub const DEFAULT_REDFISH_TIMEOUT_SECONDS: u64 = 30;
const LOCAL_TEST_NONCE: &str = "000102030405060708090a0b0c0d0e0f101112131415161718191a1b1c1d1e1f";

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

pub struct InventoryOptions {
    pub config: String,
    pub target: Option<String>,
    pub port: u16,
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

pub async fn run_inventory(options: InventoryOptions) -> Result<(), Box<dyn Error>> {
    let mut targets = config::load_targets(&options.config)?;
    if let Some(target_name) = &options.target {
        targets.retain(|target| target.name == *target_name);
        if targets.is_empty() {
            return Err(format!("target {target_name:?} not found").into());
        }
    }

    let timeout = std::time::Duration::from_secs(options.timeout_seconds);
    let source = RedfishSource::new(options.port, options.insecure_tls, timeout)?;
    let mapper = RedfishMapper;
    let concurrency = options.concurrency.max(1);

    let inventories: Vec<_> = stream::iter(targets)
        .map(|target| async {
            let record = source.collect(target).await;
            mapper.map(record)
        })
        .buffered(concurrency)
        .collect()
        .await;

    match options.format {
        InventoryFormat::Table => TableSink::new(options.detail).write(&inventories)?,
        InventoryFormat::Json => JsonSink.write(&inventories)?,
    }
    Ok(())
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
                        let mut client = match HttpBackendClient::new(
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
                    let sink = BackendSink::new(
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
