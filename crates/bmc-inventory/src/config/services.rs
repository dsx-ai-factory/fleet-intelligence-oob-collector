// Copyright (c) 2026, NVIDIA CORPORATION & AFFILIATES. All rights reserved.
// SPDX-License-Identifier: Apache-2.0

//! Typed NICo collection controls shared by rendering and configuration reports.

use std::error::Error;
use std::num::NonZeroU32;

use serde::{Deserialize, Serialize};

/// Explicit overrides; omitted fields retain the existing NICo behavior.
#[derive(Clone, Debug, Default, Deserialize, Serialize)]
#[serde(default, deny_unknown_fields, rename_all(serialize = "camelCase"))]
pub struct NicoHealthSettings {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub sensor_fetch_interval_seconds: Option<NonZeroU32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub bmc_request_concurrency: Option<NonZeroU32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub endpoint_discovery_interval_seconds: Option<NonZeroU32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub discovery: Option<NicoDiscoverySettings>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub sensors: Option<NicoSensorSettings>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub metrics: Option<NicoFetchSettings>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub telemetry: Option<NicoFetchSettings>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub firmware: Option<NicoFirmwareSettings>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub manager: Option<NicoPollSettings>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub leak_detector: Option<NicoLeakDetectorSettings>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub logs: Option<NicoLogSettings>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub reachability: Option<NicoReachabilitySettings>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub rate_limit: Option<NicoRateLimitSettings>,
}

/// Explicit overrides; omitted fields retain the existing NICo behavior.
#[derive(Clone, Debug, Default, Deserialize, Serialize)]
#[serde(default, deny_unknown_fields, rename_all(serialize = "camelCase"))]
pub struct NicoDiscoverySettings {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub refresh_interval_seconds: Option<NonZeroU32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub concurrency: Option<NonZeroU32>,
}

/// Explicit overrides; omitted fields retain the existing NICo behavior.
#[derive(Clone, Debug, Default, Deserialize, Serialize)]
#[serde(default, deny_unknown_fields, rename_all(serialize = "camelCase"))]
pub struct NicoSensorSettings {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub enabled: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub include_sensor_thresholds: Option<bool>,
}

/// Explicit overrides; omitted fields retain the existing NICo behavior.
#[derive(Clone, Debug, Default, Deserialize, Serialize)]
#[serde(default, deny_unknown_fields, rename_all(serialize = "camelCase"))]
pub struct NicoFetchSettings {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub enabled: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub fetch_interval_seconds: Option<NonZeroU32>,
}

/// Explicit overrides; omitted fields retain the existing NICo behavior.
#[derive(Clone, Debug, Default, Deserialize, Serialize)]
#[serde(default, deny_unknown_fields, rename_all(serialize = "camelCase"))]
pub struct NicoFirmwareSettings {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub enabled: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub refresh_interval_seconds: Option<NonZeroU32>,
}

/// Explicit overrides; omitted fields retain the existing NICo behavior.
#[derive(Clone, Debug, Default, Deserialize, Serialize)]
#[serde(default, deny_unknown_fields, rename_all(serialize = "camelCase"))]
pub struct NicoPollSettings {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub enabled: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub poll_interval_seconds: Option<NonZeroU32>,
}

/// Explicit overrides; omitted fields retain the existing NICo behavior.
#[derive(Clone, Debug, Default, Deserialize, Serialize)]
#[serde(default, deny_unknown_fields, rename_all(serialize = "camelCase"))]
pub struct NicoLeakDetectorSettings {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub enabled: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub poll_interval_seconds: Option<NonZeroU32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub state_refresh_interval_seconds: Option<NonZeroU32>,
}

/// Explicit overrides; omitted fields retain the existing NICo behavior.
#[derive(Clone, Debug, Default, Deserialize, Serialize)]
#[serde(default, deny_unknown_fields, rename_all(serialize = "camelCase"))]
pub struct NicoLogSettings {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub enabled: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub mode: Option<NicoLogMode>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub sse: Option<NicoSseSettings>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub periodic: Option<NicoPeriodicLogSettings>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub auto: Option<NicoAutoLogSettings>,
}

/// Explicit overrides; omitted fields retain the existing NICo behavior.
#[derive(Clone, Debug, Default, Deserialize, Serialize)]
#[serde(default, deny_unknown_fields, rename_all(serialize = "camelCase"))]
pub struct NicoSseSettings {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub initial_backoff_seconds: Option<NonZeroU32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub max_backoff_seconds: Option<NonZeroU32>,
}

/// Explicit overrides; omitted fields retain the existing NICo behavior.
#[derive(Clone, Debug, Default, Deserialize, Serialize)]
#[serde(default, deny_unknown_fields, rename_all(serialize = "camelCase"))]
pub struct NicoPeriodicLogSettings {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub collection_interval_seconds: Option<NonZeroU32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub state_refresh_interval_seconds: Option<NonZeroU32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub exclude_services: Option<Vec<String>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub skip_initial_history: Option<bool>,
}

/// Explicit overrides; omitted fields retain the existing NICo behavior.
#[derive(Clone, Debug, Default, Deserialize, Serialize)]
#[serde(default, deny_unknown_fields, rename_all(serialize = "camelCase"))]
pub struct NicoAutoLogSettings {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub sse_not_available_threshold: Option<NonZeroU32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub connect_failure_window_seconds: Option<NonZeroU32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub connect_failure_threshold: Option<NonZeroU32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub retry_sse_after_downgrade: Option<bool>,
}

/// Explicit overrides; omitted fields retain the existing NICo behavior.
#[derive(Clone, Debug, Default, Deserialize, Serialize)]
#[serde(default, deny_unknown_fields, rename_all(serialize = "camelCase"))]
pub struct NicoReachabilitySettings {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub enabled: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub interval_seconds: Option<NonZeroU32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub timeout_seconds: Option<NonZeroU32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub log_mode: Option<NicoReachabilityLogMode>,
}

/// Explicit overrides; omitted fields retain the existing NICo behavior.
#[derive(Clone, Debug, Default, Deserialize, Serialize)]
#[serde(default, deny_unknown_fields, rename_all(serialize = "camelCase"))]
pub struct NicoRateLimitSettings {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub enabled: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub bucket_burst: Option<NonZeroU32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub bucket_replenish_milliseconds: Option<NonZeroU32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub max_jitter_milliseconds: Option<u32>,
}

/// Supported NICo collection mode.
#[derive(Clone, Copy, Debug, Deserialize, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum NicoLogMode {
    Sse,
    Periodic,
    Auto,
}

/// Supported NICo collection mode.
#[derive(Clone, Copy, Debug, Deserialize, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum NicoReachabilityLogMode {
    All,
    Unreachable,
}

impl NicoHealthSettings {
    /// Validate cross-field constraints before rendering or reporting settings.
    pub(super) fn validate(&self) -> Result<(), Box<dyn Error>> {
        if let Some(concurrency) = self.bmc_request_concurrency
            && u64::from(concurrency.get()) > tokio::sync::Semaphore::MAX_PERMITS as u64
        {
            return Err(
                "nico_health.bmc_request_concurrency exceeds NICo's semaphore limit".into(),
            );
        }
        if let Some(sse) = self.logs.as_ref().and_then(|logs| logs.sse.as_ref())
            && sse.max_backoff_seconds.map_or(30, NonZeroU32::get)
                < sse.initial_backoff_seconds.map_or(1, NonZeroU32::get)
        {
            return Err(
                "nico_health.logs.sse.max_backoff_seconds must be at least initial_backoff_seconds"
                    .into(),
            );
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reject_invalid_nested_types_values_modes_and_unknown_fields() {
        for input in [
            "[leak_detector]\npoll_interval_seconds = 0",
            "[discovery]\nconcurrency = -1",
            "[logs.sse]\nmax_backoff_seconds = 4294967296",
            "[metrics]\nenabled = 'false'",
            "[logs]\nmode = 'invalid'",
            "[logs.auto]\nconnect_failure_threshold = 0",
            "[reachability]\nlog_mode = 'invalid'",
            "[reachability]\ntimeout_seconds = 1.5",
            "[rate_limit]\nmax_jitter_milliseconds = -1",
            "[logs.periodic]\nexclude_services = [false]",
            "[leak_detector]\npoll_intervl_seconds = 60",
            "[logs.sse]\npassword = 'never-report-this'",
        ] {
            assert!(
                toml::from_str::<NicoHealthSettings>(input).is_err(),
                "{input}"
            );
        }
        for input in [
            "[logs.sse]\ninitial_backoff_seconds = 31",
            "[logs.sse]\ninitial_backoff_seconds = 10\nmax_backoff_seconds = 9",
        ] {
            let settings: NicoHealthSettings = toml::from_str(input).unwrap();
            assert!(settings.validate().is_err());
        }
    }

    #[test]
    fn nested_reports_preserve_explicit_false_zero_and_empty_arrays() {
        let settings: NicoHealthSettings = toml::from_str(include_str!(
            "../../tests/fixtures/nico-health-settings.toml"
        ))
        .unwrap();
        let json = serde_json::to_value(settings).unwrap();
        assert_eq!(json["leakDetector"]["enabled"], false);
        assert_eq!(
            json["logs"]["periodic"]["excludeServices"],
            serde_json::json!([])
        );
        assert_eq!(json["rateLimit"]["maxJitterMilliseconds"], 0);
        assert_eq!(json["discovery"]["refreshIntervalSeconds"], 600);
        assert_eq!(json["logs"]["auto"]["retrySseAfterDowngrade"], false);
        assert_eq!(
            serde_json::to_value(NicoHealthSettings::default()).unwrap(),
            serde_json::json!({})
        );
    }
}
