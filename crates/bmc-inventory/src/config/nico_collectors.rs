// Copyright (c) 2026, NVIDIA CORPORATION & AFFILIATES. All rights reserved.
// SPDX-License-Identifier: Apache-2.0

//! Render flat operator settings into NICo's native TOML sections.

use std::num::NonZeroU32;

use toml::{Table, Value};

use super::services::{NicoHealthSettings, NicoLogMode, NicoReachabilityLogMode};

fn set<T: Into<Value>>(table: &mut Table, key: &str, value: Option<T>) {
    if let Some(value) = value {
        table.insert(key.into(), value.into());
    }
}

fn number(table: &mut Table, key: &str, value: Option<NonZeroU32>) {
    set(table, key, value.map(|n| i64::from(n.get())));
}

fn seconds(table: &mut Table, key: &str, value: Option<NonZeroU32>) {
    set(table, key, value.map(|n| format!("{n}s")));
}

/// Timing overrides alone must preserve each collector's existing enablement.
fn add_group(collectors: &mut Table, name: &str, mut table: Table, default: Option<bool>) {
    if table.is_empty() {
        return;
    }
    if let Some(enabled) = default {
        table.entry("enabled").or_insert(Value::Boolean(enabled));
    }
    collectors.insert(name.into(), table.into());
}

impl NicoHealthSettings {
    /// Build optional polling parameters shared by periodic mode and auto fallback.
    fn native_periodic_logs(&self) -> Table {
        let mut table = Table::new();
        seconds(
            &mut table,
            "logs_collection_interval",
            self.logs_collection_interval_seconds,
        );
        seconds(
            &mut table,
            "state_refresh_interval",
            self.logs_state_refresh_interval_seconds,
        );
        set(
            &mut table,
            "skip_initial_history",
            self.logs_skip_initial_history,
        );
        set(
            &mut table,
            "exclude_services",
            self.logs_exclude_services.as_ref().map(|services| {
                Value::Array(services.iter().cloned().map(Value::String).collect())
            }),
        );
        table
    }

    /// Build NICo sections from flat fields, defaulting to auto logs with SSE recovery.
    pub(super) fn native_collectors(&self) -> Table {
        let mut collectors = Table::new();
        let mut table = Table::new();
        seconds(
            &mut table,
            "refresh_interval",
            self.discovery_refresh_interval_seconds
                .or(NonZeroU32::new(3600)),
        );
        number(
            &mut table,
            "discovery_concurrency",
            self.discovery_concurrency,
        );
        add_group(&mut collectors, "discovery", table, None);
        let mut table = Table::new();
        set(&mut table, "enabled", self.sensors_enabled);
        seconds(
            &mut table,
            "sensor_fetch_interval",
            self.sensor_fetch_interval_seconds.or(NonZeroU32::new(60)),
        );
        set(
            &mut table,
            "include_sensor_thresholds",
            self.include_sensor_thresholds,
        );
        add_group(&mut collectors, "sensors", table, Some(true));
        let mut table = Table::new();
        set(&mut table, "enabled", self.metrics_enabled);
        seconds(
            &mut table,
            "fetch_interval",
            self.metrics_fetch_interval_seconds,
        );
        add_group(&mut collectors, "metrics", table, Some(false));
        let mut table = Table::new();
        set(&mut table, "enabled", self.telemetry_enabled);
        seconds(
            &mut table,
            "fetch_interval",
            self.telemetry_fetch_interval_seconds,
        );
        add_group(&mut collectors, "telemetry", table, Some(false));
        let mut table = Table::new();
        set(&mut table, "enabled", self.firmware_enabled);
        seconds(
            &mut table,
            "firmware_refresh_interval",
            self.firmware_refresh_interval_seconds,
        );
        add_group(&mut collectors, "firmware", table, Some(false));
        let mut table = Table::new();
        set(&mut table, "enabled", self.manager_enabled);
        seconds(
            &mut table,
            "poll_interval",
            self.manager_poll_interval_seconds,
        );
        add_group(&mut collectors, "manager", table, Some(true));
        let mut table = Table::new();
        set(&mut table, "enabled", self.leak_detector_enabled);
        seconds(
            &mut table,
            "poll_interval",
            self.leak_detector_poll_interval_seconds,
        );
        seconds(
            &mut table,
            "state_refresh_interval",
            self.leak_detector_state_refresh_interval_seconds,
        );
        add_group(&mut collectors, "leak_detector", table, Some(true));
        let mut table = Table::new();
        set(&mut table, "enabled", self.reachability_enabled);
        seconds(
            &mut table,
            "interval",
            self.reachability_interval_seconds.or(NonZeroU32::new(60)),
        );
        seconds(&mut table, "timeout", self.reachability_timeout_seconds);
        set(
            &mut table,
            "log_mode",
            self.reachability_log_mode.map(|mode| match mode {
                NicoReachabilityLogMode::All => "all",
                NicoReachabilityLogMode::Unreachable => "unreachable",
            }),
        );
        add_group(&mut collectors, "reachability", table, Some(true));
        let mut logs = Table::new();
        logs.insert(
            "enabled".into(),
            Value::Boolean(self.logs_enabled.unwrap_or(true)),
        );
        let mode = self.logs_mode.unwrap_or(NicoLogMode::Auto);
        logs.insert(
            "mode".into(),
            Value::String(
                match mode {
                    NicoLogMode::Sse => "sse",
                    NicoLogMode::Periodic => "periodic",
                    NicoLogMode::Auto => "auto",
                }
                .into(),
            ),
        );
        let mut sse = Table::new();
        seconds(
            &mut sse,
            "initial_backoff",
            self.logs_sse_initial_backoff_seconds,
        );
        seconds(&mut sse, "max_backoff", self.logs_sse_max_backoff_seconds);
        if !sse.is_empty() {
            logs.insert("sse".into(), sse.into());
        }
        let periodic = self.native_periodic_logs();
        if !periodic.is_empty() || matches!(mode, NicoLogMode::Periodic) {
            logs.insert("periodic".into(), periodic.clone().into());
        }
        let mut auto = Table::new();
        number(
            &mut auto,
            "sse_not_available_threshold",
            self.logs_auto_sse_not_available_threshold,
        );
        seconds(
            &mut auto,
            "connect_failure_window",
            self.logs_auto_connect_failure_window_seconds,
        );
        number(
            &mut auto,
            "connect_failure_threshold",
            self.logs_auto_connect_failure_threshold,
        );
        set(
            &mut auto,
            "retry_sse_after_downgrade",
            self.logs_auto_retry_sse_after_downgrade.or(Some(true)),
        );
        if !auto.is_empty() {
            // NICo prefers the auto table's flattened polling parameters, so
            // carry the same explicit polling controls into its fallback path.
            auto.extend(periodic);
            logs.insert("auto".into(), auto.into());
        }
        collectors.insert("logs".into(), logs.into());
        collectors
    }

    /// The global limiter admits iterations independently of per-BMC operation slots.
    pub(super) fn native_rate_limit(&self) -> Option<Table> {
        let mut table = Table::new();
        set(&mut table, "enabled", self.rate_limit_enabled);
        number(&mut table, "bucket_burst", self.rate_limit_bucket_burst);
        set(
            &mut table,
            "bucket_replenish",
            self.rate_limit_bucket_replenish_milliseconds
                .map(|n| format!("{n}ms")),
        );
        set(
            &mut table,
            "max_jitter",
            self.rate_limit_max_jitter_milliseconds
                .map(|n| format!("{n}ms")),
        );
        if table.is_empty() {
            return None;
        }
        table.entry("enabled").or_insert(Value::Boolean(false));
        Some(table)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const SETTINGS: &str = include_str!("../../tests/fixtures/nico-health-settings.toml");

    #[test]
    fn all_collection_controls_map_to_native_names_and_units() {
        let settings: NicoHealthSettings = toml::from_str(SETTINGS).unwrap();
        settings.validate().unwrap();
        let expected: Table = toml::from_str(
            r#"
[discovery]
refresh_interval = "600s"
discovery_concurrency = 8
[sensors]
enabled = false
sensor_fetch_interval = "120s"
include_sensor_thresholds = false
[metrics]
enabled = true
fetch_interval = "180s"
[telemetry]
enabled = true
fetch_interval = "90s"
[firmware]
enabled = true
firmware_refresh_interval = "14400s"
[manager]
enabled = false
poll_interval = "600s"
[leak_detector]
enabled = false
poll_interval = "120s"
state_refresh_interval = "3600s"
[logs]
enabled = true
mode = "auto"
[logs.sse]
initial_backoff = "2s"
max_backoff = "60s"
[logs.periodic]
logs_collection_interval = "600s"
state_refresh_interval = "3600s"
exclude_services = []
skip_initial_history = true
[logs.auto]
logs_collection_interval = "600s"
state_refresh_interval = "3600s"
exclude_services = []
skip_initial_history = true
sse_not_available_threshold = 2
connect_failure_window = "600s"
connect_failure_threshold = 8
retry_sse_after_downgrade = false
[reachability]
enabled = true
interval = "60s"
timeout = "2s"
log_mode = "all"
"#,
        )
        .unwrap();
        assert_eq!(settings.native_collectors(), expected);
        let rate: Table = toml::from_str(
            r#"
enabled = true
bucket_burst = 50
bucket_replenish = "100ms"
max_jitter = "0ms"
"#,
        )
        .unwrap();
        assert_eq!(settings.native_rate_limit(), Some(rate));
    }

    #[test]
    fn tuning_alone_preserves_each_collector_enablement() {
        let settings: NicoHealthSettings = toml::from_str(
            r#"
metrics_fetch_interval_seconds = 180
telemetry_fetch_interval_seconds = 90
firmware_refresh_interval_seconds = 14400
manager_poll_interval_seconds = 600
leak_detector_poll_interval_seconds = 120
reachability_interval_seconds = 60
rate_limit_bucket_burst = 50
"#,
        )
        .unwrap();
        let collectors = settings.native_collectors();
        for name in ["metrics", "telemetry", "firmware"] {
            assert_eq!(collectors[name]["enabled"].as_bool(), Some(false), "{name}");
        }
        for name in ["manager", "leak_detector", "logs", "reachability"] {
            assert_eq!(collectors[name]["enabled"].as_bool(), Some(true), "{name}");
        }
        assert_eq!(
            settings.native_rate_limit().unwrap()["enabled"].as_bool(),
            Some(false)
        );
    }

    #[test]
    fn log_mode_and_recovery_defaults_preserve_independent_overrides() {
        for (input, mode, recovery) in [
            ("", "auto", true),
            ("logs_mode = 'sse'", "sse", true),
            ("logs_mode = 'periodic'", "periodic", true),
            ("logs_auto_retry_sse_after_downgrade = false", "auto", false),
        ] {
            let settings: NicoHealthSettings = toml::from_str(input).unwrap();
            let collectors = settings.native_collectors();
            assert_eq!(collectors["logs"]["mode"].as_str(), Some(mode), "{input}");
            assert_eq!(
                collectors["logs"]["auto"]["retry_sse_after_downgrade"].as_bool(),
                Some(recovery),
                "{input}"
            );
        }
    }

    #[test]
    fn periodic_mode_generates_required_settings_without_extra_overrides() {
        let settings: NicoHealthSettings = toml::from_str("logs_mode = 'periodic'").unwrap();
        let collectors = settings.native_collectors();
        assert_eq!(collectors["logs"]["periodic"].as_table().unwrap().len(), 0);
        assert!(collectors["logs"].get("sse").is_none());
        let settings: NicoHealthSettings = toml::from_str("logs_enabled = false").unwrap();
        assert_eq!(
            settings.native_collectors()["logs"]["enabled"].as_bool(),
            Some(false)
        );
    }
}
