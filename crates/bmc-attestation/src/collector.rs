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
use crate::sink::{BackendClient, BackendHttpError, HttpBackendClient};
use crate::source::{MeasurementRequest, NonceData, RedfishAttestationSource, now_rfc3339};

/// Result of running the attestation pipeline over a batch of targets.
pub struct CollectionOutcome {
    /// The batch ready to upload (failures are captured as failed node records).
    pub batch: BatchAttestation,
    /// A rotated agent token, if the backend issued one on a nonce response, so the
    /// caller can refresh its session and authenticate the upload with it.
    pub refreshed_jwt: Option<String>,
}

/// Whether a backend error is a JWT rejection (HTTP 401/403), signalling that the
/// agent must re-enroll rather than record per-target failures with a dead token.
fn is_auth_failure(err: &(dyn Error + 'static)) -> bool {
    err.downcast_ref::<BackendHttpError>()
        .is_some_and(|err| matches!(err.status(), 401 | 403))
}

/// Collect attestation evidence for every target concurrently.
///
/// Per target: build a backend client from `jwt`, request a nonce, collect signed
/// measurements over Redfish, and map them. A non-auth target-level failure
/// (client/nonce error) becomes a failed `NodeAttestation` rather than aborting the
/// batch. A backend auth failure (HTTP 401/403) is propagated as a batch-level
/// error instead, so the caller can re-enroll and retry the whole batch with a
/// fresh token rather than uploading failures collected under a rejected JWT.
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

    type TargetOutcome = Result<(NodeAttestation, Option<String>), Box<dyn Error>>;

    let collected: Vec<TargetOutcome> = stream::iter(targets)
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
                        return Ok((
                            NodeAttestation::failed(
                                node_uuid,
                                target.name,
                                target.address,
                                format!("backend client: {err}"),
                            ),
                            None,
                        ));
                    }
                };
                let nonce = match client.get_nonce(&node_uuid).await {
                    Ok(response) => response,
                    // A rejected JWT cannot be recovered per target: abort the batch so
                    // the caller can re-enroll and retry with a fresh token.
                    Err(err) if is_auth_failure(err.as_ref()) => return Err(err),
                    Err(err) => {
                        return Ok((
                            NodeAttestation::failed(
                                node_uuid,
                                target.name,
                                target.address,
                                format!("nonce request: {err}"),
                            ),
                            None,
                        ));
                    }
                };
                // The backend may rotate the agent token on a nonce response.
                let refreshed_jwt = nonce
                    .jwt_assertion
                    .clone()
                    .filter(|token| !token.is_empty());
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
                Ok((mapper.map(record), refreshed_jwt))
            }
        })
        .buffered(concurrency)
        .collect()
        .await;

    let mut attestations = Vec::with_capacity(collected.len());
    let mut refreshed_jwt = None;
    for outcome in collected {
        let (record, refreshed) = outcome?;
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn classifies_jwt_rejections_as_auth_failures() {
        for status in [401, 403] {
            let err: Box<dyn Error> = Box::new(BackendHttpError::new(status, "denied"));
            assert!(
                is_auth_failure(err.as_ref()),
                "status {status} should be auth failure"
            );
        }
    }

    #[test]
    fn other_errors_are_not_auth_failures() {
        let http: Box<dyn Error> = Box::new(BackendHttpError::new(500, "boom"));
        assert!(!is_auth_failure(http.as_ref()));

        let plain: Box<dyn Error> = "nonce request: timeout".into();
        assert!(!is_auth_failure(plain.as_ref()));
    }
}
