// Copyright (c) 2026, NVIDIA CORPORATION & AFFILIATES. All rights reserved.
// SPDX-License-Identifier: Apache-2.0

//! Explicit allowlist of configurable settings reported with inventory.
//! Never serialize target credentials, generated NICo configuration, or Helm values wholesale.

use std::error::Error;

use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CollectorReport {
    pub runtime: RuntimeReport,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub nico: Option<NicoReport>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub telemetry: Option<TelemetryReport>,
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RuntimeReport {
    pub inventory_interval_seconds: u64,
    pub target_concurrency: usize,
    pub bmc_max_in_flight: u32,
    pub bmc_requests_per_second: u64,
    pub bmc_burst: u64,
    pub timeout_seconds: u64,
    pub insecure_tls: bool,
    pub latency_probe: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub log_level: Option<String>,
}

/// Helm supplies this file separately from the BMC target configuration.
/// Missing sections mean unknown, whereas `enabled: false` means configured off.
#[derive(Clone, Debug, Default, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct DeploymentReport {
    pub nico: Option<NicoReport>,
    pub telemetry: Option<TelemetryReport>,
}

impl DeploymentReport {
    pub fn load(path: &str) -> Result<Self, Box<dyn Error>> {
        let contents = std::fs::read_to_string(path)?;
        // Parse errors can contain input excerpts; do not expose arbitrary file contents.
        serde_json::from_str(&contents)
            .map_err(|_| "invalid deployment reporting configuration".into())
    }
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct NicoReport {
    pub enabled: bool,
    pub image_tag: String,
    pub metrics_prefix: String,
    pub metrics_port: u16,
    pub service_enabled: bool,
    pub tls_enabled: bool,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct TelemetryReport {
    pub enabled: bool,
    pub image_tag: String,
    pub replicas: u32,
    pub grpc_port: u16,
    pub http_port: u16,
    pub termination_grace_period_seconds: u64,
    pub network_policy_enabled: bool,
    pub memory_limiter: MemoryLimiterReport,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct MemoryLimiterReport {
    pub limit_percentage: u8,
    pub spike_limit_percentage: u8,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn absent_deployment_settings_are_unknown_not_disabled() {
        let deployment: DeploymentReport = serde_json::from_str("{}").unwrap();
        assert!(deployment.nico.is_none());
        assert!(deployment.telemetry.is_none());
    }

    #[test]
    fn report_preserves_explicit_false_and_excludes_fixed_nico_settings() {
        let deployment: DeploymentReport = serde_json::from_value(serde_json::json!({
            "nico": {
                "enabled": false,
                "imageTag": "test",
                "metricsPrefix": "custom",
                "metricsPort": 9010,
                "serviceEnabled": false,
                "tlsEnabled": false
            }
        }))
        .unwrap();
        let value = serde_json::to_value(deployment.nico.unwrap()).unwrap();
        assert_eq!(value["enabled"], false);
        assert_eq!(value["serviceEnabled"], false);
        assert_eq!(value["metricsPrefix"], "custom");
        assert_eq!(value.as_object().unwrap().len(), 6);
        assert!(value.get("logs").is_none());
        assert!(value.get("batchSize").is_none());
    }

    #[test]
    fn reporting_file_rejects_fields_outside_the_allowlist() {
        for field in ["password", "agentToken", "otlpEndpoint", "runtime"] {
            let value = serde_json::json!({field: "do-not-export"});
            assert!(serde_json::from_value::<DeploymentReport>(value).is_err());
        }
        let value = serde_json::json!({
            "nico": {
                "enabled": true,
                "imageTag": "test",
                "metricsPrefix": "test",
                "metricsPort": 9009,
                "serviceEnabled": true,
                "tlsEnabled": false,
                "password": "do-not-export"
            }
        });
        assert!(serde_json::from_value::<DeploymentReport>(value).is_err());
    }
}
