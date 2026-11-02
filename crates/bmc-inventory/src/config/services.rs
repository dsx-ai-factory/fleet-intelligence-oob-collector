// Copyright (c) 2026, NVIDIA CORPORATION & AFFILIATES. All rights reserved.
// SPDX-License-Identifier: Apache-2.0

//! Typed NICo collection controls shared by rendering and configuration reports.

use std::error::Error;
use std::num::NonZeroU32;

use serde::{Deserialize, Serialize};

/// Flat NICo controls shared by Helm/native configuration and backend reporting.
/// Omitted fields preserve existing behavior.
#[derive(Clone, Debug, Default, Deserialize, Serialize)]
#[serde(default, deny_unknown_fields, rename_all(serialize = "camelCase"))]
pub struct NicoHealthSettings {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub sensor_fetch_interval_seconds: Option<NonZeroU32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub bmc_request_concurrency: Option<NonZeroU32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub bmc_latency_metrics_enabled: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub bmc_latency_attributes: Option<Vec<String>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub endpoint_discovery_interval_seconds: Option<NonZeroU32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub discovery_refresh_interval_seconds: Option<NonZeroU32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub discovery_concurrency: Option<NonZeroU32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub sensors_enabled: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub include_sensor_thresholds: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub metrics_enabled: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub metrics_fetch_interval_seconds: Option<NonZeroU32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub telemetry_enabled: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub telemetry_fetch_interval_seconds: Option<NonZeroU32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub firmware_enabled: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub firmware_refresh_interval_seconds: Option<NonZeroU32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub manager_enabled: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub manager_poll_interval_seconds: Option<NonZeroU32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub leak_detector_enabled: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub leak_detector_poll_interval_seconds: Option<NonZeroU32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub leak_detector_state_refresh_interval_seconds: Option<NonZeroU32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub logs_enabled: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub logs_mode: Option<NicoLogMode>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub logs_sse_initial_backoff_seconds: Option<NonZeroU32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub logs_sse_max_backoff_seconds: Option<NonZeroU32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub logs_collection_interval_seconds: Option<NonZeroU32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub logs_state_refresh_interval_seconds: Option<NonZeroU32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub logs_exclude_services: Option<Vec<String>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub logs_skip_initial_history: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub logs_auto_sse_not_available_threshold: Option<NonZeroU32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub logs_auto_connect_failure_window_seconds: Option<NonZeroU32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub logs_auto_connect_failure_threshold: Option<NonZeroU32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub logs_auto_retry_sse_after_downgrade: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub reachability_enabled: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub reachability_interval_seconds: Option<NonZeroU32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub reachability_timeout_seconds: Option<NonZeroU32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub reachability_log_mode: Option<NicoReachabilityLogMode>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub rate_limit_enabled: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub rate_limit_bucket_burst: Option<NonZeroU32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub rate_limit_bucket_replenish_milliseconds: Option<NonZeroU32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub rate_limit_max_jitter_milliseconds: Option<u32>,
}

/// Supported NICo collection mode.
#[derive(Clone, Copy, Debug, Deserialize, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum NicoLogMode {
    Sse,
    Periodic,
    Auto,
}

/// Reachability probe logging mode.
#[derive(Clone, Copy, Debug, Deserialize, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum NicoReachabilityLogMode {
    All,
    Unreachable,
}

impl NicoHealthSettings {
    /// Validate cross-field constraints before rendering or reporting settings.
    pub(super) fn validate(&self) -> Result<(), Box<dyn Error>> {
        if self
            .logs_sse_max_backoff_seconds
            .map_or(30, NonZeroU32::get)
            < self
                .logs_sse_initial_backoff_seconds
                .map_or(1, NonZeroU32::get)
        {
            return Err("nico_health.logs_sse_max_backoff_seconds must be at least logs_sse_initial_backoff_seconds".into());
        }
        if let Some(attributes) = &self.bmc_latency_attributes {
            const ALLOWED: &[&str] = &[
                "all",
                "http_response_status_code",
                "http_request_method",
                "http_path",
                "server_address",
                "url_scheme",
                "bmc_vendor",
                "bmc_model",
                "entity_type",
                "machine_id",
                "rack_id",
            ];
            if attributes.is_empty()
                || attributes
                    .iter()
                    .any(|attribute| !ALLOWED.contains(&attribute.as_str()))
            {
                return Err("nico_health.bmc_latency_attributes must contain at least one supported BMC latency attribute".into());
            }
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reject_invalid_flat_types_values_modes_and_unknown_fields() {
        for input in [
            "leak_detector_poll_interval_seconds = 0",
            "discovery_concurrency = -1",
            "logs_sse_max_backoff_seconds = 4294967296",
            "metrics_enabled = 'false'",
            "bmc_latency_metrics_enabled = 'true'",
            "bmc_latency_attributes = [false]",
            "logs_mode = 'invalid'",
            "logs_auto_connect_failure_threshold = 0",
            "reachability_log_mode = 'invalid'",
            "reachability_timeout_seconds = 1.5",
            "rate_limit_max_jitter_milliseconds = -1",
            "logs_exclude_services = [false]",
            "leak_detector_poll_intervl_seconds = 60",
            "password = 'never-report-this'",
        ] {
            assert!(
                toml::from_str::<NicoHealthSettings>(input).is_err(),
                "{input}"
            );
        }
        for input in [
            "logs_sse_initial_backoff_seconds = 31",
            "logs_sse_initial_backoff_seconds = 10\nlogs_sse_max_backoff_seconds = 9",
            "bmc_latency_attributes = []",
            "bmc_latency_metrics_enabled = false\nbmc_latency_attributes = []",
            "bmc_latency_attributes = ['unsupported']",
            "bmc_latency_attributes = ['all', 'unsupported']",
        ] {
            let settings: NicoHealthSettings = toml::from_str(input).unwrap();
            assert!(settings.validate().is_err());
        }
    }

    #[test]
    fn flat_reports_preserve_explicit_false_zero_and_empty_arrays() {
        let settings: NicoHealthSettings = toml::from_str(include_str!(
            "../../tests/fixtures/nico-health-settings.toml"
        ))
        .unwrap();
        let json = serde_json::to_value(settings).unwrap();
        assert_eq!(json["leakDetectorEnabled"], false);
        assert_eq!(json["bmcLatencyMetricsEnabled"], false);
        assert_eq!(json["logsExcludeServices"], serde_json::json!([]));
        assert_eq!(json["rateLimitMaxJitterMilliseconds"], 0);
        assert_eq!(json["discoveryRefreshIntervalSeconds"], 600);
        assert_eq!(json["logsAutoRetrySseAfterDowngrade"], false);
        assert_eq!(
            serde_json::to_value(NicoHealthSettings::default()).unwrap(),
            serde_json::json!({})
        );
    }
}
