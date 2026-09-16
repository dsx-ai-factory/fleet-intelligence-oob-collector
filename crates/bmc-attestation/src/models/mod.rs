use std::collections::BTreeMap;

use bmc_error::sanitize_backend_error_message;
use serde::Serialize;

pub const ATTESTATION_SCHEMA_VERSION: &str = "attestation.v1alpha1";

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct BatchAttestation {
    pub schema_version: String,
    pub collected_at: String,
    pub results_by_node_uuid: BTreeMap<String, NodeAttestation>,
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct NodeAttestation {
    pub node_uuid: String,
    pub target_name: String,
    pub bmc_address: String,
    pub success: bool,
    pub error_message: Option<String>,
    pub nonce_refresh_timestamp: Option<String>,
    pub evidences: Vec<AttestationEvidence>,
    pub component_errors: Vec<ComponentError>,
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AttestationEvidence {
    pub component_id: String,
    pub arch: Option<String>,
    pub vbios_version: Option<String>,
    pub nonce: String,
    pub evidence: String,
    pub certificate: String,
    pub version: Option<String>,
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ComponentError {
    pub component_id: Option<String>,
    pub message: String,
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct BackendBatchRequest {
    pub schema_version: String,
    pub collected_at: String,
    pub attestations: BTreeMap<String, BackendNodeAttestation>,
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct BackendNodeAttestation {
    pub nonce_refresh_timestamp: Option<String>,
    pub success: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error_message: Option<String>,
    pub evidences: Vec<BackendEvidenceItem>,
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct BackendEvidenceItem {
    pub arch: Option<String>,
    pub certificate: String,
    pub driver_version: Option<String>,
    pub evidence: String,
    pub nonce: String,
    pub vbios_version: Option<String>,
    pub version: Option<String>,
}

impl BatchAttestation {
    pub fn new(collected_at: String, results: Vec<NodeAttestation>) -> Self {
        let results_by_node_uuid = results
            .into_iter()
            .map(|result| (result.node_uuid.clone(), result))
            .collect();
        Self {
            schema_version: ATTESTATION_SCHEMA_VERSION.to_string(),
            collected_at,
            results_by_node_uuid,
        }
    }

    pub fn backend_request(&self) -> BackendBatchRequest {
        BackendBatchRequest {
            schema_version: self.schema_version.clone(),
            collected_at: self.collected_at.clone(),
            attestations: self
                .results_by_node_uuid
                .iter()
                .map(|(node_uuid, result)| (node_uuid.clone(), result.backend_payload()))
                .collect(),
        }
    }
}

impl NodeAttestation {
    pub fn failed(
        node_uuid: String,
        target_name: String,
        bmc_address: String,
        message: impl Into<String>,
    ) -> Self {
        Self {
            node_uuid,
            target_name,
            bmc_address,
            success: false,
            error_message: Some(message.into()),
            nonce_refresh_timestamp: None,
            evidences: Vec::new(),
            component_errors: Vec::new(),
        }
    }

    fn backend_payload(&self) -> BackendNodeAttestation {
        BackendNodeAttestation {
            nonce_refresh_timestamp: self.nonce_refresh_timestamp.clone(),
            success: self.success,
            // Redact absolute URLs before upload to avoid request body filters.
            error_message: self
                .error_message
                .as_deref()
                .map(sanitize_backend_error_message),
            evidences: self
                .evidences
                .iter()
                .map(|evidence| BackendEvidenceItem {
                    arch: evidence.arch.clone(),
                    certificate: evidence.certificate.clone(),
                    driver_version: None,
                    evidence: evidence.evidence.clone(),
                    nonce: evidence.nonce.clone(),
                    vbios_version: evidence.vbios_version.clone(),
                    version: evidence.version.clone(),
                })
                .collect(),
        }
    }
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;

    #[test]
    fn serializes_batch_keyed_by_node_uuid() {
        let batch = BatchAttestation::new(
            "2026-01-01T00:00:00Z".to_string(),
            vec![NodeAttestation {
                node_uuid: "node-1".to_string(),
                target_name: "node-a".to_string(),
                bmc_address: "10.0.0.1".to_string(),
                success: true,
                error_message: None,
                nonce_refresh_timestamp: Some("2026-01-01T00:00:00Z".to_string()),
                evidences: vec![AttestationEvidence {
                    component_id: "GPU_0".to_string(),
                    arch: Some("BLACKWELL".to_string()),
                    vbios_version: Some("97.00.DD.00.01".to_string()),
                    nonce: "nonce".to_string(),
                    evidence: "signed-measurements".to_string(),
                    certificate: "cert-b64".to_string(),
                    version: Some("1.1.0".to_string()),
                }],
                component_errors: Vec::new(),
            }],
        );

        let value = serde_json::to_value(batch).expect("json");

        assert_eq!(value["schemaVersion"], "attestation.v1alpha1");
        assert_eq!(
            value["resultsByNodeUuid"]["node-1"]["evidences"][0]["arch"],
            "BLACKWELL"
        );
    }

    #[test]
    fn backend_payload_uses_node_uuid_keys() {
        let batch = BatchAttestation::new(
            "2026-01-01T00:00:00Z".to_string(),
            vec![NodeAttestation::failed(
                "node-1".to_string(),
                "node-a".to_string(),
                "10.0.0.1".to_string(),
                "timeout",
            )],
        );

        let value = serde_json::to_value(batch.backend_request()).expect("json");

        assert_eq!(
            value,
            json!({
                "schemaVersion": "attestation.v1alpha1",
                "collectedAt": "2026-01-01T00:00:00Z",
                "attestations": {
                    "node-1": {
                        "nonceRefreshTimestamp": null,
                        "success": false,
                        "errorMessage": "timeout",
                        "evidences": []
                    }
                }
            })
        );
    }

    #[test]
    fn backend_payload_strips_reqwest_url_from_error_message() {
        let batch = BatchAttestation::new(
            "2026-01-01T00:00:00Z".to_string(),
            vec![NodeAttestation::failed(
                "node-1".to_string(),
                "node-a".to_string(),
                "10.91.54.137".to_string(),
                "HTTP status client error (404 Not Found) for url (https://10.91.54.137/redfish/v1/ComponentIntegrity)",
            )],
        );

        let value = serde_json::to_value(batch.backend_request()).expect("json");
        assert_eq!(
            value["attestations"]["node-1"]["errorMessage"],
            "HTTP status client error (404 Not Found) for url (/redfish/v1/ComponentIntegrity)"
        );
        // Local record keeps the full message for debugging.
        assert!(
            batch.results_by_node_uuid["node-1"]
                .error_message
                .as_deref()
                .unwrap()
                .contains("https://10.91.54.137/")
        );
    }

    #[test]
    fn backend_payload_sanitizes_scheme_relative_error_urls() {
        let record = NodeAttestation::failed(
            "node-1".into(),
            "node-a".into(),
            "10.0.0.1".into(),
            "failed //user:secret@host/redfish/v1/ComponentIntegrity?token=x#item",
        );
        let value = serde_json::to_value(record.backend_payload()).expect("serialize");
        assert_eq!(
            value["errorMessage"],
            "failed /redfish/v1/ComponentIntegrity"
        );
        assert!(record.error_message.as_ref().unwrap().contains("secret"));
    }

    #[test]
    fn sanitize_backend_error_message_preserves_embedded_url_paths() {
        assert_eq!(
            sanitize_backend_error_message(
                "fetch failed contacting https://10.0.0.1/redfish/v1/ComponentIntegrity"
            ),
            "fetch failed contacting /redfish/v1/ComponentIntegrity"
        );
        assert_eq!(
            sanitize_backend_error_message("no GPU IRoT ComponentIntegrity resources found"),
            "no GPU IRoT ComponentIntegrity resources found"
        );
    }

    #[test]
    fn sanitize_backend_error_message_preserves_paths_for_multiple_urls() {
        let sanitized = sanitize_backend_error_message(
            "proxy https://10.0.0.1/redfish/v1 failed: HTTP status client error (404 Not Found) for url (https://10.91.54.137/redfish/v1/ComponentIntegrity)",
        );
        assert_eq!(
            sanitized,
            "proxy /redfish/v1 failed: HTTP status client error (404 Not Found) for url (/redfish/v1/ComponentIntegrity)"
        );
        assert!(!sanitized.contains("http://") && !sanitized.contains("https://"));
    }
}
