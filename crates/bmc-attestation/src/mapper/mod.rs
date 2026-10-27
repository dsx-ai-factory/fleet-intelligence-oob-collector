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

use bmc_error::SanitizedErrorMessage;

use crate::models::{AttestationEvidence, ComponentError, NodeAttestation};
use crate::source::{AttestationEvidenceData, ComponentErrorData, SourceRecord};

pub trait AttestationMapper {
    fn map(&self, record: SourceRecord) -> NodeAttestation;
}

#[derive(Clone, Copy, Debug, Default)]
pub struct RedfishMapper;

impl AttestationMapper for RedfishMapper {
    fn map(&self, record: SourceRecord) -> NodeAttestation {
        let target = record.target;
        match record.result {
            Ok(report) => {
                let success = !report.evidences.is_empty();
                NodeAttestation {
                    bmc_mac: target.bmc_mac,
                    target_name: target.name,
                    bmc_address: target.address,
                    success,
                    error_message: if success {
                        None
                    } else {
                        Some(
                            "failed to collect attestation evidence from all GPU IRoT components"
                                .into(),
                        )
                    },
                    nonce_refresh_timestamp: record.nonce_refresh_timestamp,
                    evidences: report.evidences.into_iter().map(map_evidence).collect(),
                    component_errors: report.errors.into_iter().map(map_component_error).collect(),
                }
            }
            Err(error) => {
                let mut node =
                    NodeAttestation::failed(target.bmc_mac, target.name, target.address, error);
                node.nonce_refresh_timestamp = record.nonce_refresh_timestamp;
                node
            }
        }
    }
}

fn map_evidence(evidence: AttestationEvidenceData) -> AttestationEvidence {
    AttestationEvidence {
        component_id: evidence.component_id,
        arch: evidence.arch,
        vbios_version: evidence.vbios_version,
        nonce: evidence.nonce,
        evidence: evidence.evidence,
        certificate: evidence.certificate,
        version: evidence.version,
    }
}

fn map_component_error(error: ComponentErrorData) -> ComponentError {
    ComponentError {
        component_id: error.component_id,
        message: SanitizedErrorMessage::new(error.message),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::Target;
    use crate::source::{RedfishAttestationReport, SourceRecord};

    #[test]
    fn maps_success_report() {
        let node = RedfishMapper.map(SourceRecord {
            target: target(),
            collected_at: "2026-01-01T00:00:00Z".to_string(),
            nonce_refresh_timestamp: Some("2026-01-01T00:00:00Z".to_string()),
            result: Ok(RedfishAttestationReport {
                evidences: vec![AttestationEvidenceData {
                    component_id: "GPU_0".to_string(),
                    arch: Some("BLACKWELL".to_string()),
                    vbios_version: Some("97.00.DD.00.01".to_string()),
                    nonce: "nonce".to_string(),
                    evidence: "evidence".to_string(),
                    certificate: "cert".to_string(),
                    version: Some("1.1.0".to_string()),
                }],
                errors: Vec::new(),
            }),
        });

        assert!(node.success);
        assert_eq!(node.bmc_mac, "00:11:22:33:44:55");
        assert_eq!(node.evidences.len(), 1);
        assert_eq!(node.evidences[0].arch.as_deref(), Some("BLACKWELL"));
    }

    #[test]
    fn sanitizes_component_errors_before_building_attestation_model() {
        let node = RedfishMapper.map(SourceRecord {
            target: target(),
            collected_at: "2026-01-01T00:00:00Z".to_string(),
            nonce_refresh_timestamp: None,
            result: Ok(RedfishAttestationReport {
                evidences: Vec::new(),
                errors: vec![ComponentErrorData {
                    component_id: Some("GPU_0".into()),
                    message:
                        "failed //user:secret@bmc.example/redfish/v1/ComponentIntegrity?token=x"
                            .into(),
                }],
            }),
        });

        assert_eq!(node.component_errors[0].message, "failed [URL_REDACTED]");
        let rendered = serde_json::to_string(&node).unwrap();
        assert!(!rendered.contains("user:secret@"));
        assert!(!rendered.contains("token=x"));
    }

    #[test]
    fn maps_error_record_as_failed_node() {
        let node = RedfishMapper.map(SourceRecord {
            target: target(),
            collected_at: "2026-01-01T00:00:00Z".to_string(),
            nonce_refresh_timestamp: Some("2026-01-01T00:00:00Z".to_string()),
            result: Err("nonce request: timeout".to_string()),
        });

        assert!(!node.success);
        assert_eq!(
            node.error_message.as_deref(),
            Some("nonce request: timeout")
        );
        assert_eq!(
            node.nonce_refresh_timestamp.as_deref(),
            Some("2026-01-01T00:00:00Z")
        );
    }

    fn target() -> Target {
        Target {
            name: "node-a".to_string(),
            bmc_mac: "00:11:22:33:44:55".to_string(),
            address: "10.0.0.1".to_string(),
            credentials_profile: None,
            username: "root".to_string(),
            password: "secret".to_string(),
        }
    }
}
