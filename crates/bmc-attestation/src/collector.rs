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

//! Attestation collection pipeline.
//!
//! Ties the `source -> mapper` shape together with the backend nonce exchange:
//! for each target it fetches a per-BMC nonce, collects signed measurements over
//! Redfish, and maps the result into a `NodeAttestation`. Backend session and
//! upload concerns are left to the caller.

use std::collections::BTreeSet;
use std::error::Error;

use backend_client::is_auth_failure;
use bmc_dispatch::Dispatcher;
use bmc_error::SanitizedErrorMessage;
use futures::stream::{self, StreamExt};

use crate::config::{AttestationConfig, Target};
use crate::mapper::{AttestationMapper, RedfishMapper};
use crate::models::{BatchAttestation, NodeAttestation};
#[cfg(test)]
use crate::sink::BackendHttpError;
use crate::sink::{BackendClient, HttpBackendClient};
use crate::source::{
    CredentialReloader, MeasurementRequest, NonceData, RedfishAttestationSource, now_rfc3339,
};

/// Result of running the attestation pipeline over a batch of targets.
pub struct CollectionOutcome {
    /// The batch ready to upload, excluding targets whose nonce request failed.
    pub batch: BatchAttestation,
    /// Targets skipped because their nonce request failed.
    pub skipped_targets: usize,
    /// A rotated agent token, if the backend issued one on a nonce response, so the
    /// caller can refresh its session and authenticate the upload with it.
    pub refreshed_jwt: Option<String>,
    /// BMCs whose collection failed in a way a later attempt this cycle could
    /// fix: rejected credentials or a transient error.
    pub retry_bmc_macs: BTreeSet<String>,
}

/// Collect attestation evidence for every target concurrently.
///
/// Per target: clone the backend client, request a nonce, collect signed
/// measurements over Redfish, and map them. Nonce errors skip that target's
/// evidence collection and upload. A backend auth failure (HTTP 401) is propagated as a batch-level
/// error instead, so the caller can re-enroll and retry the whole batch with a
/// fresh token rather than uploading failures collected under a rejected JWT.
pub async fn collect(
    targets: &[Target],
    settings: &AttestationConfig,
    backend: &HttpBackendClient,
    dispatcher: &Dispatcher,
    reload_credentials: &CredentialReloader<'_>,
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
    // Each per-target nonce client starts from the current token; any rotation the
    // backend returns is captured below for the caller to apply.

    type TargetOutcome = Result<(Option<NodeAttestation>, Option<String>, bool), Box<dyn Error>>;

    let collected: Vec<TargetOutcome> = stream::iter(targets)
        .map(|target| {
            let source = source.clone();
            let measurement = measurement.clone();
            let mut client = backend.clone();
            let target = target.clone();
            let bmc_mac = target.bmc_mac.clone();
            let gate = dispatcher.attestation_gate(&target.name);
            async move {
                let Some(gate) = gate else {
                    return Ok((
                        Some(NodeAttestation::failed(
                            bmc_mac,
                            target.name,
                            target.address,
                            "no dispatcher endpoint configured for attestation",
                        )),
                        None,
                        false,
                    ));
                };
                let nonce = match client.get_nonce(&bmc_mac).await {
                    Ok(response) => response,
                    // A rejected JWT cannot be recovered per target: abort the batch so
                    // the caller can re-enroll and retry with a fresh token.
                    Err(err) if is_auth_failure(err.as_ref()) => return Err(err),
                    Err(err) => {
                        tracing::warn!(
                            %bmc_mac,
                            error = %SanitizedErrorMessage::new(err.to_string()),
                            "nonce request failed; skipping attestation for target"
                        );
                        return Ok((None, None, false));
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
                        gate,
                        reload_credentials,
                    )
                    .await;
                let retryable = record.retryable;
                Ok((Some(mapper.map(record)), refreshed_jwt, retryable))
            }
        })
        .buffered(concurrency)
        .collect()
        .await;

    let mut attestations = Vec::with_capacity(collected.len());
    let mut refreshed_jwt = None;
    let mut skipped_targets = 0;
    let mut retry_bmc_macs = BTreeSet::new();
    for outcome in collected {
        let (record, refreshed, retryable) = outcome?;
        if retryable && let Some(record) = &record {
            retry_bmc_macs.insert(record.bmc_mac.clone());
        }
        if refreshed.is_some() {
            refreshed_jwt = refreshed;
        }
        if let Some(record) = record {
            attestations.push(record);
        } else {
            skipped_targets += 1;
        }
    }

    Ok(CollectionOutcome {
        batch: BatchAttestation::new(now_rfc3339(), attestations),
        skipped_targets,
        refreshed_jwt,
        retry_bmc_macs,
    })
}

#[cfg(test)]
mod tests {
    use bmc_dispatch::DispatcherConfig;
    use std::time::Duration;

    use super::*;

    #[test]
    fn classifies_unauthorized_as_auth_failure() {
        let err: Box<dyn Error> = Box::new(BackendHttpError::new(401, "denied"));
        assert!(is_auth_failure(err.as_ref()));
    }

    #[test]
    fn other_errors_are_not_auth_failures() {
        let forbidden: Box<dyn Error> = Box::new(BackendHttpError::new(403, "blocked by WAF"));
        assert!(!is_auth_failure(forbidden.as_ref()));

        let http: Box<dyn Error> = Box::new(BackendHttpError::new(500, "boom"));
        assert!(!is_auth_failure(http.as_ref()));

        let plain: Box<dyn Error> = "nonce request: timeout".into();
        assert!(!is_auth_failure(plain.as_ref()));
    }

    #[tokio::test]
    async fn missing_dispatcher_endpoint_fails_before_backend_request() {
        let dispatcher = Dispatcher::start(
            DispatcherConfig {
                global_max_in_flight: 1,
                bmc_max_in_flight: 1,
                bmc_requests_per_second: 100,
                bmc_burst: 100,
                workflow_queue_capacity: 8,
            },
            ["different-bmc".to_string()],
        )
        .expect("dispatcher");
        let target = Target {
            name: "bmc-1".to_string(),
            bmc_mac: "00:11:22:33:44:55".to_string(),
            address: "192.0.2.1".to_string(),
            credentials_profile: Some("bmc".to_string()),
            username: "user".to_string(),
            password: "secret".into(),
        };

        let outcome = collect(
            &[target],
            &AttestationConfig::default(),
            &HttpBackendClient::new(
                "https://backend.invalid",
                "/nonce/{bmc_mac}",
                "/attestation",
                "token",
                Duration::from_secs(1),
            )
            .unwrap(),
            &dispatcher,
            &|_| panic!("missing gate"),
        )
        .await
        .expect("missing gate becomes a target failure");
        let record = outcome
            .batch
            .results_by_bmc_mac
            .get("00:11:22:33:44:55")
            .expect("target result");

        assert!(!record.success);
        assert_eq!(
            record.error_message.as_deref(),
            Some("no dispatcher endpoint configured for attestation")
        );
    }
}
