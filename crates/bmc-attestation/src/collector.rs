//! Attestation collection pipeline.
//!
//! Ties the `source -> mapper` shape together with the backend nonce exchange:
//! for each target it fetches a per-node nonce, collects signed measurements over
//! Redfish, and maps the result into a `NodeAttestation`. Backend session and
//! upload concerns are left to the caller.

use std::error::Error;

use futures::stream::{self, StreamExt};

use crate::config::{AttestationConfig, Target};
use crate::mapper::{AttestationMapper, RedfishMapper};
use crate::models::{BatchAttestation, NodeAttestation};
use crate::sink::{BackendClient, HttpBackendClient};
use crate::source::{MeasurementRequest, NonceData, RedfishAttestationSource, now_rfc3339};

/// Result of running the attestation pipeline over a batch of targets.
pub struct CollectionOutcome {
    /// The batch ready to upload (failures are captured as failed node records).
    pub batch: BatchAttestation,
    /// A rotated agent token, if the backend issued one on a nonce response, so the
    /// caller can refresh its session and authenticate the upload with it.
    pub refreshed_jwt: Option<String>,
}

/// Collect attestation evidence for every target concurrently.
///
/// Per target: build a backend client from `jwt`, request a nonce, collect signed
/// measurements over Redfish, and map them. A target-level failure (client/nonce
/// error) becomes a failed `NodeAttestation` rather than aborting the batch.
pub async fn collect(
    targets: &[Target],
    settings: &AttestationConfig,
    backend_url: &str,
    nonce_path: &str,
    attestation_path: &str,
    jwt: &str,
) -> Result<CollectionOutcome, Box<dyn Error>> {
    let source = RedfishAttestationSource::new(
        settings.port,
        settings.insecure_tls,
        settings.redfish_timeout,
        settings.task_timeout,
        settings.task_poll_interval,
    )?;
    let mapper = RedfishMapper;
    let measurement = MeasurementRequest {
        measurement_indices: settings.measurement_indices.clone(),
    };
    let concurrency = settings.concurrency.max(1);
    let timeout = settings.redfish_timeout;
    // Each per-target nonce client starts from the current token; any rotation the
    // backend returns is captured below for the caller to apply.
    let base_jwt = jwt.to_string();

    let collected: Vec<(NodeAttestation, Option<String>)> = stream::iter(targets)
        .map(|target| {
            let source = source.clone();
            let measurement = measurement.clone();
            let backend_url = backend_url.to_string();
            let nonce_path = nonce_path.to_string();
            let attestation_path = attestation_path.to_string();
            let jwt = base_jwt.clone();
            let target = target.clone();
            let node_uuid = target.node_uuid.clone();
            async move {
                let mut client = match HttpBackendClient::new(
                    &backend_url,
                    &nonce_path,
                    &attestation_path,
                    jwt,
                    timeout,
                ) {
                    Ok(client) => client,
                    Err(err) => {
                        return (
                            NodeAttestation::failed(
                                node_uuid,
                                target.name,
                                target.address,
                                format!("backend client: {err}"),
                            ),
                            None,
                        );
                    }
                };
                let nonce = match client.get_nonce(&node_uuid).await {
                    Ok(response) => response,
                    Err(err) => {
                        return (
                            NodeAttestation::failed(
                                node_uuid,
                                target.name,
                                target.address,
                                format!("nonce request: {err}"),
                            ),
                            None,
                        );
                    }
                };
                // The backend may rotate the agent token on a nonce response.
                let refreshed_jwt = nonce.jwt_assertion.clone().filter(|token| !token.is_empty());
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

    let mut attestations = Vec::with_capacity(collected.len());
    let mut refreshed_jwt = None;
    for (record, refreshed) in collected {
        if refreshed.is_some() {
            refreshed_jwt = refreshed;
        }
        attestations.push(record);
    }

    Ok(CollectionOutcome {
        batch: BatchAttestation::new(now_rfc3339(), attestations),
        refreshed_jwt,
    })
}
