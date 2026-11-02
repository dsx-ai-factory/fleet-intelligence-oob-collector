// Copyright (c) 2026, NVIDIA CORPORATION & AFFILIATES. All rights reserved.
// SPDX-License-Identifier: Apache-2.0

//! Map typed collection controls to NICo-native TOML, preserving enablement defaults.

use std::num::NonZeroU32;

use toml::{Table, Value};

use super::services::{
    NicoHealthSettings, NicoLogMode, NicoPeriodicLogSettings, NicoReachabilityLogMode,
};

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

fn enabled(value: Option<bool>, default: bool) -> Table {
    let mut table = Table::new();
    table.insert("enabled".into(), Value::Boolean(value.unwrap_or(default)));
    table
}

fn periodic_logs(settings: &NicoPeriodicLogSettings) -> Table {
    let mut table = Table::new();
    seconds(
        &mut table,
        "logs_collection_interval",
        settings.collection_interval_seconds,
    );
    seconds(
        &mut table,
        "state_refresh_interval",
        settings.state_refresh_interval_seconds,
    );
    set(
        &mut table,
        "skip_initial_history",
        settings.skip_initial_history,
    );
    set(
        &mut table,
        "exclude_services",
        settings
            .exclude_services
            .as_ref()
            .map(|services| Value::Array(services.iter().cloned().map(Value::String).collect())),
    );
    table
}

impl NicoHealthSettings {
    /// Emit only explicitly selected groups, plus the existing SSE log defaults.
    pub(super) fn native_collectors(&self) -> Table {
        let mut collectors = Table::new();
        if let Some(settings) = &self.discovery {
            let mut table = Table::new();
            seconds(
                &mut table,
                "refresh_interval",
                settings.refresh_interval_seconds,
            );
            number(&mut table, "discovery_concurrency", settings.concurrency);
            collectors.insert("discovery".into(), table.into());
        }
        if self.sensors.is_some() || self.sensor_fetch_interval_seconds.is_some() {
            let settings = self.sensors.clone().unwrap_or_default();
            let mut table = enabled(settings.enabled, true);
            seconds(
                &mut table,
                "sensor_fetch_interval",
                self.sensor_fetch_interval_seconds,
            );
            set(
                &mut table,
                "include_sensor_thresholds",
                settings.include_sensor_thresholds,
            );
            collectors.insert("sensors".into(), table.into());
        }
        for (name, settings) in [("metrics", &self.metrics), ("telemetry", &self.telemetry)] {
            if let Some(settings) = settings {
                let mut table = enabled(settings.enabled, false);
                seconds(
                    &mut table,
                    "fetch_interval",
                    settings.fetch_interval_seconds,
                );
                collectors.insert(name.into(), table.into());
            }
        }
        if let Some(settings) = &self.firmware {
            let mut table = enabled(settings.enabled, false);
            seconds(
                &mut table,
                "firmware_refresh_interval",
                settings.refresh_interval_seconds,
            );
            collectors.insert("firmware".into(), table.into());
        }
        if let Some(settings) = &self.manager {
            let mut table = enabled(settings.enabled, true);
            seconds(&mut table, "poll_interval", settings.poll_interval_seconds);
            collectors.insert("manager".into(), table.into());
        }
        if let Some(settings) = &self.leak_detector {
            let mut table = enabled(settings.enabled, true);
            seconds(&mut table, "poll_interval", settings.poll_interval_seconds);
            seconds(
                &mut table,
                "state_refresh_interval",
                settings.state_refresh_interval_seconds,
            );
            collectors.insert("leak_detector".into(), table.into());
        }
        if let Some(settings) = &self.reachability {
            let mut table = enabled(settings.enabled, false);
            seconds(&mut table, "interval", settings.interval_seconds);
            seconds(&mut table, "timeout", settings.timeout_seconds);
            set(
                &mut table,
                "log_mode",
                settings.log_mode.map(|mode| match mode {
                    NicoReachabilityLogMode::All => "all",
                    NicoReachabilityLogMode::Unreachable => "unreachable",
                }),
            );
            collectors.insert("reachability".into(), table.into());
        }
        let settings = self.logs.clone().unwrap_or_default();
        let mut logs = enabled(settings.enabled, true);
        let mode = settings.mode.unwrap_or(NicoLogMode::Sse);
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
        if let Some(sse) = &settings.sse {
            let mut table = Table::new();
            seconds(&mut table, "initial_backoff", sse.initial_backoff_seconds);
            seconds(&mut table, "max_backoff", sse.max_backoff_seconds);
            logs.insert("sse".into(), table.into());
        }
        let periodic = settings.periodic.as_ref().map(periodic_logs);
        if periodic.is_some() || matches!(mode, NicoLogMode::Periodic) {
            logs.insert(
                "periodic".into(),
                periodic.clone().unwrap_or_default().into(),
            );
        }
        if let Some(auto) = &settings.auto {
            // NICo prefers the auto table's flattened polling settings over the
            // periodic table. Copy explicit polling overrides to both paths.
            let mut table = periodic.unwrap_or_default();
            number(
                &mut table,
                "sse_not_available_threshold",
                auto.sse_not_available_threshold,
            );
            seconds(
                &mut table,
                "connect_failure_window",
                auto.connect_failure_window_seconds,
            );
            number(
                &mut table,
                "connect_failure_threshold",
                auto.connect_failure_threshold,
            );
            set(
                &mut table,
                "retry_sse_after_downgrade",
                auto.retry_sse_after_downgrade,
            );
            logs.insert("auto".into(), table.into());
        }
        collectors.insert("logs".into(), logs.into());
        collectors
    }

    /// Interval limiter controls iterations, independently of per-BMC operation slots.
    pub(super) fn native_rate_limit(&self) -> Option<Table> {
        let settings = self.rate_limit.as_ref()?;
        let mut table = enabled(settings.enabled, false);
        number(&mut table, "bucket_burst", settings.bucket_burst);
        set(
            &mut table,
            "bucket_replenish",
            settings
                .bucket_replenish_milliseconds
                .map(|n| format!("{n}ms")),
        );
        set(
            &mut table,
            "max_jitter",
            settings.max_jitter_milliseconds.map(|n| format!("{n}ms")),
        );
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
[metrics]
fetch_interval_seconds = 180
[telemetry]
fetch_interval_seconds = 90
[firmware]
refresh_interval_seconds = 14400
[reachability]
interval_seconds = 60
[manager]
poll_interval_seconds = 600
[leak_detector]
poll_interval_seconds = 120
[rate_limit]
bucket_burst = 50
"#,
        )
        .unwrap();
        let collectors = settings.native_collectors();
        for name in ["metrics", "telemetry", "firmware", "reachability"] {
            assert_eq!(collectors[name]["enabled"].as_bool(), Some(false), "{name}");
        }
        for name in ["manager", "leak_detector", "logs"] {
            assert_eq!(collectors[name]["enabled"].as_bool(), Some(true), "{name}");
        }
        assert_eq!(
            settings.native_rate_limit().unwrap()["enabled"].as_bool(),
            Some(false)
        );
    }

    #[test]
    fn periodic_mode_generates_required_settings_without_extra_overrides() {
        let settings: NicoHealthSettings = toml::from_str("[logs]\nmode = 'periodic'").unwrap();
        let collectors = settings.native_collectors();
        assert_eq!(collectors["logs"]["periodic"].as_table().unwrap().len(), 0);
        assert!(collectors["logs"].get("sse").is_none());
        let settings: NicoHealthSettings = toml::from_str("[logs]\nenabled = false").unwrap();
        assert_eq!(
            settings.native_collectors()["logs"]["enabled"].as_bool(),
            Some(false)
        );
    }
}
