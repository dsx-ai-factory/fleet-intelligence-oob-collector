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

use std::collections::BTreeMap;
use std::error::Error;
use std::net::IpAddr;
use std::num::NonZeroU32;
use std::str::FromStr;

use mac_address::MacAddress;
use serde::Serialize;
use toml::Value;

use super::CollectorConfig;

#[derive(Clone, Debug)]
pub struct NicoConfigOptions {
    pub otlp_endpoint: String,
    pub metrics_endpoint: String,
    pub metrics_prefix: String,
    pub resource_status_collection: bool,
    pub resource_status_firmware_poll_interval_seconds: u64,
}

impl Default for NicoConfigOptions {
    fn default() -> Self {
        Self {
            otlp_endpoint: "http://localhost:4317".to_string(),
            metrics_endpoint: "0.0.0.0:9009".to_string(),
            metrics_prefix: "fleetint".to_string(),
            resource_status_collection: true,
            resource_status_firmware_poll_interval_seconds: 1_800,
        }
    }
}

/// Render shared targets and NICo defaults with explicit overrides, without embedding credentials.
pub fn render_nico_config(
    config: &CollectorConfig,
    options: &NicoConfigOptions,
) -> Result<String, Box<dyn Error>> {
    config.nico_health.validate()?;
    let otlp_endpoint = options.otlp_endpoint.trim();
    let metrics_endpoint = options.metrics_endpoint.trim();
    let metrics_prefix = options.metrics_prefix.trim();
    if otlp_endpoint.is_empty() {
        return Err("NICo OTLP endpoint cannot be empty".into());
    }
    if metrics_endpoint.is_empty() {
        return Err("NICo metrics endpoint cannot be empty".into());
    }
    if metrics_prefix.is_empty() {
        return Err("NICo metrics prefix cannot be empty".into());
    }
    if options.resource_status_firmware_poll_interval_seconds == 0 {
        return Err("NICo resource status firmware poll interval must be greater than zero".into());
    }

    let static_bmc_endpoints = config
        .targets
        .iter()
        .map(|target| NicoStaticBmcEndpoint {
            ip: target.ip,
            port: target.port,
            mac: target.mac.to_string(),
            credentials_profile: target.credentials_profile.as_deref(),
            labels: target.labels.clone(),
        })
        .collect();

    let mut collectors = config.nico_health.native_collectors();
    if options.resource_status_collection {
        // Firmware status comes from the firmware collector, so the status
        // feature takes precedence over independently configured firmware
        // enablement and cadence while it is active.
        let firmware = collectors
            .entry("firmware")
            .or_insert_with(|| Value::Table(toml::Table::new()))
            .as_table_mut()
            .ok_or("internal NICo firmware collector configuration is not a table")?;
        firmware.insert("enabled".into(), Value::Boolean(true));
        firmware.insert(
            "firmware_refresh_interval".into(),
            Value::String(format!(
                "{}s",
                options.resource_status_firmware_poll_interval_seconds
            )),
        );
    }

    let nico_config = NicoConfig {
        bmc_request_concurrency: config
            .nico_health
            .bmc_request_concurrency
            .or(NonZeroU32::new(2)),
        endpoint_discovery_interval: config
            .nico_health
            .endpoint_discovery_interval_seconds
            .map(|n| format!("{n}s")),
        rate_limit: config.nico_health.native_rate_limit(),
        endpoint_sources: NicoEndpointSources {
            nico_api: NicoEnabled { enabled: false },
            static_credentials_file: &config.credentials_file,
            static_bmc_endpoints,
        },
        collectors,
        processors: NicoProcessors {
            resource_status: NicoResourceStatusProcessor {
                enabled: options.resource_status_collection,
            },
        },
        attributes: NicoAttributes { gpu_identity: true },
        sinks: NicoSinks {
            tracing: NicoEnabled { enabled: true },
            health_report: NicoEnabled { enabled: false },
            rack_health_report: NicoEnabled { enabled: false },
            switch_health_report: NicoEnabled { enabled: false },
            power_shelf_health_report: NicoEnabled { enabled: false },
            prometheus: NicoEnabled { enabled: true },
            otlp: NicoOtlpSink {
                enabled: true,
                targets: vec![NicoOtlpTarget {
                    endpoint: otlp_endpoint,
                    include_diagnostics: true,
                    include_alert_details: true,
                    batch_size: 512,
                    flush_interval: "2s",
                }],
            },
        },
        metrics: NicoMetrics {
            endpoint: metrics_endpoint,
            prefix: metrics_prefix,
        },
    };

    toml::to_string_pretty(&nico_config)
        .map_err(|error| format!("failed to render NICo TOML config: {error}").into())
}

/// Add FI-owned collection and telemetry settings to an existing NICo configuration.
///
/// Endpoint sources and unrelated customer settings remain customer-owned. Known FI
/// collection controls override the corresponding native collector fields, while
/// unknown collector and plugin fields are retained.
pub fn merge_nico_config(
    base: &str,
    config: &CollectorConfig,
    options: &NicoConfigOptions,
) -> Result<String, Box<dyn Error>> {
    let mut merged: Value =
        toml::from_str(base).map_err(|_| "existing NICo configuration is not valid TOML")?;
    let overlay: Value = toml::from_str(&render_nico_config(config, options)?)
        .map_err(|_| "internally rendered NICo configuration is not valid TOML")?;

    let merged_root = merged
        .as_table_mut()
        .ok_or("existing NICo configuration root must be a TOML table")?;
    let overlay_root = overlay
        .as_table()
        .ok_or("internally rendered NICo configuration root is not a TOML table")?;

    // These controls are already part of the FI Helm contract. Overlay their
    // known fields without deleting customer-only collectors or nested fields.
    for key in [
        "bmc_request_concurrency",
        "endpoint_discovery_interval",
        "rate_limit",
    ] {
        if let Some(value) = overlay_root.get(key) {
            merged_root.insert(key.into(), value.clone());
        }
    }
    merge_root_table(merged_root, overlay_root, "collectors")?;
    merge_nested_table(
        merged_root,
        overlay_root,
        &["processors", "resource_status"],
    )?;
    merge_nested_table(merged_root, overlay_root, &["attributes"])?;
    merge_static_endpoint_labels(merged_root, overlay_root)?;
    merge_fi_otlp_target(merged_root, overlay_root)?;

    toml::to_string_pretty(&merged)
        .map_err(|error| format!("failed to serialize merged NICo configuration: {error}").into())
}

fn merge_root_table(
    merged: &mut toml::Table,
    overlay: &toml::Table,
    key: &str,
) -> Result<(), Box<dyn Error>> {
    let Some(overlay_value) = overlay.get(key) else {
        return Ok(());
    };
    let overlay_table = overlay_value
        .as_table()
        .ok_or_else(|| format!("internal NICo {key} configuration is not a table"))?;
    let merged_value = merged
        .entry(key)
        .or_insert_with(|| Value::Table(toml::Table::new()));
    let merged_table = merged_value
        .as_table_mut()
        .ok_or_else(|| format!("existing NICo {key} configuration must be a table"))?;
    deep_merge(merged_table, overlay_table);
    Ok(())
}

fn merge_nested_table(
    merged: &mut toml::Table,
    overlay: &toml::Table,
    path: &[&str],
) -> Result<(), Box<dyn Error>> {
    let overlay_table = table_at_path(overlay, path, "internal")?;
    let merged_table = table_at_path_mut(merged, path, "existing")?;
    deep_merge(merged_table, overlay_table);
    Ok(())
}

fn table_at_path<'a>(
    mut table: &'a toml::Table,
    path: &[&str],
    owner: &str,
) -> Result<&'a toml::Table, Box<dyn Error>> {
    for key in path {
        table = table.get(*key).and_then(Value::as_table).ok_or_else(|| {
            format!(
                "{owner} NICo {} configuration must be a table",
                path.join(".")
            )
        })?;
    }
    Ok(table)
}

fn table_at_path_mut<'a>(
    mut table: &'a mut toml::Table,
    path: &[&str],
    owner: &str,
) -> Result<&'a mut toml::Table, Box<dyn Error>> {
    for key in path {
        let value = table
            .entry(*key)
            .or_insert_with(|| Value::Table(toml::Table::new()));
        table = value.as_table_mut().ok_or_else(|| {
            format!(
                "{owner} NICo {} configuration must be a table",
                path.join(".")
            )
        })?;
    }
    Ok(table)
}

fn deep_merge(destination: &mut toml::Table, overlay: &toml::Table) {
    for (key, overlay_value) in overlay {
        match (
            destination.get_mut(key).and_then(Value::as_table_mut),
            overlay_value.as_table(),
        ) {
            (Some(destination_table), Some(overlay_table)) => {
                deep_merge(destination_table, overlay_table);
            }
            _ => {
                destination.insert(key.clone(), overlay_value.clone());
            }
        }
    }
}

fn merge_static_endpoint_labels(
    merged: &mut toml::Table,
    overlay: &toml::Table,
) -> Result<(), Box<dyn Error>> {
    let overlay_sources = table_at_path(overlay, &["endpoint_sources"], "internal")?;
    let overlay_endpoints = overlay_sources
        .get("static_bmc_endpoints")
        .and_then(Value::as_array)
        .ok_or("internal NICo static endpoint configuration must be an array")?;
    let labels_by_mac = endpoint_labels_by_mac(overlay_endpoints, "internal")?;

    let Some(merged_sources_value) = merged.get_mut("endpoint_sources") else {
        return Ok(());
    };
    let merged_sources = merged_sources_value
        .as_table_mut()
        .ok_or("existing NICo endpoint_sources configuration must be a table")?;
    let Some(merged_endpoints_value) = merged_sources.get_mut("static_bmc_endpoints") else {
        return Ok(());
    };
    let merged_endpoints = merged_endpoints_value
        .as_array_mut()
        .ok_or("existing NICo endpoint_sources.static_bmc_endpoints must be an array")?;

    for (index, endpoint) in merged_endpoints.iter_mut().enumerate() {
        let endpoint = endpoint.as_table_mut().ok_or_else(|| {
            format!("existing NICo endpoint_sources.static_bmc_endpoints[{index}] must be a table")
        })?;
        let Some(mac) = endpoint.get("mac").and_then(Value::as_str) else {
            continue;
        };
        let mac = normalize_mac(mac).map_err(|_| {
            format!("existing NICo endpoint_sources.static_bmc_endpoints[{index}].mac is invalid")
        })?;
        let Some(fi_labels) = labels_by_mac.get(&mac) else {
            continue;
        };
        let labels = endpoint
            .entry("labels")
            .or_insert_with(|| Value::Table(toml::Table::new()))
            .as_table_mut()
            .ok_or_else(|| {
                format!(
                    "existing NICo endpoint_sources.static_bmc_endpoints[{index}].labels must be a table"
                )
            })?;
        deep_merge(labels, fi_labels);
    }
    Ok(())
}

fn endpoint_labels_by_mac(
    endpoints: &[Value],
    owner: &str,
) -> Result<BTreeMap<String, toml::Table>, Box<dyn Error>> {
    let mut labels_by_mac = BTreeMap::new();
    for (index, endpoint) in endpoints.iter().enumerate() {
        let endpoint = endpoint.as_table().ok_or_else(|| {
            format!("{owner} NICo endpoint_sources.static_bmc_endpoints[{index}] must be a table")
        })?;
        let mac = endpoint.get("mac").and_then(Value::as_str).ok_or_else(|| {
            format!(
                "{owner} NICo endpoint_sources.static_bmc_endpoints[{index}].mac must be a string"
            )
        })?;
        let mac = normalize_mac(mac).map_err(|_| {
            format!("{owner} NICo endpoint_sources.static_bmc_endpoints[{index}].mac is invalid")
        })?;
        let labels = endpoint
            .get("labels")
            .and_then(Value::as_table)
            .ok_or_else(|| {
                format!(
                    "{owner} NICo endpoint_sources.static_bmc_endpoints[{index}].labels must be a table"
                )
            })?;
        if labels_by_mac.insert(mac, labels.clone()).is_some() {
            return Err("FI target configuration contains a duplicate BMC MAC".into());
        }
    }
    Ok(labels_by_mac)
}

fn normalize_mac(value: &str) -> Result<String, Box<dyn Error>> {
    Ok(MacAddress::from_str(value.trim())?.to_string())
}

fn merge_fi_otlp_target(
    merged: &mut toml::Table,
    overlay: &toml::Table,
) -> Result<(), Box<dyn Error>> {
    let overlay_otlp = table_at_path(overlay, &["sinks", "otlp"], "internal")?;
    let overlay_target = overlay_otlp
        .get("targets")
        .and_then(Value::as_array)
        .and_then(|targets| targets.first())
        .and_then(Value::as_table)
        .ok_or("internal NICo FI OTLP target is missing")?;
    let fi_endpoint = overlay_target
        .get("endpoint")
        .and_then(Value::as_str)
        .ok_or("internal NICo FI OTLP endpoint is missing")?;

    let merged_sinks = table_at_path_mut(merged, &["sinks"], "existing")?;
    let merged_otlp = table_at_path_mut(merged_sinks, &["otlp"], "existing")?;
    merged_otlp.insert("enabled".into(), Value::Boolean(true));
    let targets = merged_otlp
        .entry("targets")
        .or_insert_with(|| Value::Array(Vec::new()))
        .as_array_mut()
        .ok_or("existing NICo sinks.otlp.targets must be an array")?;

    let mut matching_index = None;
    for (index, target) in targets.iter().enumerate() {
        let target = target
            .as_table()
            .ok_or_else(|| format!("existing NICo sinks.otlp.targets[{index}] must be a table"))?;
        if target.get("endpoint").and_then(Value::as_str) == Some(fi_endpoint)
            && matching_index.replace(index).is_some()
        {
            return Err("existing NICo configuration repeats the FI OTLP endpoint".into());
        }
    }
    if let Some(index) = matching_index {
        let target = targets[index]
            .as_table_mut()
            .ok_or("existing NICo FI OTLP target must be a table")?;
        deep_merge(target, overlay_target);
    } else {
        targets.push(Value::Table(overlay_target.clone()));
    }
    Ok(())
}

#[derive(Serialize)]
struct NicoConfig<'a> {
    #[serde(skip_serializing_if = "Option::is_none")]
    bmc_request_concurrency: Option<NonZeroU32>,
    endpoint_sources: NicoEndpointSources<'a>,
    #[serde(skip_serializing_if = "Option::is_none")]
    endpoint_discovery_interval: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    rate_limit: Option<toml::Table>,
    collectors: toml::Table,
    processors: NicoProcessors,
    attributes: NicoAttributes,
    sinks: NicoSinks<'a>,
    metrics: NicoMetrics<'a>,
}

#[derive(Serialize)]
struct NicoEndpointSources<'a> {
    nico_api: NicoEnabled,
    static_credentials_file: &'a std::path::Path,
    static_bmc_endpoints: Vec<NicoStaticBmcEndpoint<'a>>,
}

#[derive(Serialize)]
struct NicoStaticBmcEndpoint<'a> {
    ip: IpAddr,
    #[serde(skip_serializing_if = "Option::is_none")]
    port: Option<u16>,
    mac: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    credentials_profile: Option<&'a str>,
    labels: BTreeMap<String, String>,
}

/// NICo's opt-in telemetry attributes.
///
/// `gpu_identity` adds `gpu_uuid`, `gpu_serial`, and `gpu_model` to GPU metrics
/// and log records, and `gpu_chassis_serial` to log records that name a GPU
/// chassis as their origin. NICo reads all of them from Redfish resources it
/// already fetches during discovery, so they cost no extra requests.
///
/// Always rendered on. NICo defaults the gate off because the labels reshape a
/// consumer's series and it cannot know whether a deployment wants that; we do,
/// and we do not expose the choice, so turning it off is a code change here
/// rather than a value an operator can set.
#[derive(Serialize)]
struct NicoAttributes {
    gpu_identity: bool,
}

#[derive(Serialize)]
struct NicoProcessors {
    resource_status: NicoResourceStatusProcessor,
}

#[derive(Serialize)]
struct NicoResourceStatusProcessor {
    enabled: bool,
}

#[derive(Serialize)]
struct NicoSinks<'a> {
    /// NICo defaults this sink off, which leaves collected health results with
    /// no path to its own logs: the OTLP sink carries alert details only, so a
    /// cycle that finds no alerts emits nothing observable.
    tracing: NicoEnabled,
    health_report: NicoEnabled,
    rack_health_report: NicoEnabled,
    switch_health_report: NicoEnabled,
    power_shelf_health_report: NicoEnabled,
    prometheus: NicoEnabled,
    otlp: NicoOtlpSink<'a>,
}

#[derive(Serialize)]
struct NicoEnabled {
    enabled: bool,
}

#[derive(Serialize)]
struct NicoOtlpSink<'a> {
    enabled: bool,
    targets: Vec<NicoOtlpTarget<'a>>,
}

#[derive(Serialize)]
struct NicoOtlpTarget<'a> {
    endpoint: &'a str,
    /// Carries the Redfish `DiagnosticData` payload, which is how CPER records
    /// reach the sink: NICo keeps the body opaque (base64 for CPER) and adds the
    /// `redfish.diagnostic_data.*` attributes, leaving decoding to consumers.
    ///
    /// NICo derives its collectors' diagnostic gate from the sinks, so this flag
    /// is what makes the log collectors attach the payload at all. It defaults
    /// off there because bodies are large and potentially sensitive; we accept
    /// both to get CPER off the BMC.
    include_diagnostics: bool,
    /// Include collected health-report alert details in OTLP logs.
    include_alert_details: bool,
    batch_size: usize,
    flush_interval: &'a str,
}

#[derive(Serialize)]
struct NicoMetrics<'a> {
    endpoint: &'a str,
    prefix: &'a str,
}

#[cfg(test)]
mod tests {
    use super::super::targets::parse_config;
    use super::*;

    fn parse_test_config(contents: &str, path: &str) -> Result<CollectorConfig, Box<dyn Error>> {
        static NEXT: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
        let file = std::env::temp_dir().join(format!(
            "fleetint-nico-credentials-{}-{}.json",
            std::process::id(),
            NEXT.fetch_add(1, std::sync::atomic::Ordering::Relaxed),
        ));
        std::fs::write(
            &file,
            r#"{"profiles":{"bmc":{"username":"example-user","password":"example-password"}}}"#,
        )?;
        let input = format!(
            "{contents}\n[credentials]\nfile = {:?}",
            file.to_str().unwrap()
        );
        let result = parse_config(&input, path);
        std::fs::remove_file(file)?;
        result
    }

    #[test]
    fn renders_nico_static_config_from_collector_config() {
        let collector = parse_test_config(
            r#"
            compute_zone = "us-west-dc1"
            node_group = "gb200-training"

            [[targets]]
            credentials_profile = "bmc"
            kind = "compute"
            name = "node-01"
            ip = "10.0.0.1"
            mac = " AA-BB-CC-DD-EE-FF "
            port = 8443
            labels = { site = "dev" }
            "#,
            "test.toml",
        )
        .expect("collector config");
        let rendered =
            render_nico_config(&collector, &NicoConfigOptions::default()).expect("NICo config");
        let rendered: toml::Value = toml::from_str(&rendered).expect("rendered TOML");
        let endpoint = &rendered["endpoint_sources"]["static_bmc_endpoints"][0];

        assert_eq!(endpoint["ip"].as_str(), Some("10.0.0.1"));
        assert_eq!(endpoint["port"].as_integer(), Some(8443));
        assert_eq!(endpoint["mac"].as_str(), Some("AA:BB:CC:DD:EE:FF"));
        assert_eq!(
            endpoint["labels"]["compute_zone"].as_str(),
            Some("us-west-dc1")
        );
        assert_eq!(
            endpoint["labels"]["node_group"].as_str(),
            Some("gb200-training")
        );
        assert_eq!(endpoint["labels"]["oob"].as_str(), Some("true"));
        assert_eq!(endpoint["labels"]["machine_name"].as_str(), Some("node-01"));
        assert_eq!(endpoint["labels"]["site"].as_str(), Some("dev"));
        assert!(endpoint.get("machine").is_none());
        assert_eq!(
            rendered["sinks"]["otlp"]["targets"]
                .as_array()
                .map(Vec::len),
            Some(1)
        );
        assert_eq!(
            rendered["sinks"]["otlp"]["targets"][0]["endpoint"].as_str(),
            Some("http://localhost:4317")
        );
        assert_eq!(
            rendered["sinks"]["otlp"]["targets"][0]["include_alert_details"].as_bool(),
            Some(true)
        );
        assert_eq!(
            rendered["sinks"]["otlp"]["targets"][0]["include_diagnostics"].as_bool(),
            Some(true)
        );
        assert_eq!(rendered["bmc_request_concurrency"].as_integer(), Some(2));
        let collectors = &rendered["collectors"];
        assert_eq!(collectors["sensors"]["enabled"].as_bool(), Some(true));
        assert_eq!(
            collectors["sensors"]["sensor_fetch_interval"].as_str(),
            Some("60s")
        );
        assert_eq!(
            collectors["discovery"]["refresh_interval"].as_str(),
            Some("3600s")
        );
        assert_eq!(collectors["reachability"]["enabled"].as_bool(), Some(true));
        assert_eq!(collectors["reachability"]["interval"].as_str(), Some("60s"));
        assert_eq!(collectors["logs"]["mode"].as_str(), Some("auto"));
        assert_eq!(
            collectors["logs"]["auto"]["retry_sse_after_downgrade"].as_bool(),
            Some(true)
        );
    }

    #[test]
    fn explicit_settings_override_bundle_collection_defaults() {
        let collector = parse_test_config(
            r#"
            [nico_health]
            bmc_request_concurrency = 1
            sensor_fetch_interval_seconds = 120
            discovery_refresh_interval_seconds = 86400
            reachability_enabled = false
            reachability_interval_seconds = 90
            logs_mode = "sse"
            logs_auto_retry_sse_after_downgrade = false

            [[targets]]
            credentials_profile = "bmc"
            ip = "192.0.2.10"
            mac = "02:00:00:00:00:01"
            "#,
            "test.toml",
        )
        .unwrap();
        let rendered = render_nico_config(&collector, &NicoConfigOptions::default()).unwrap();
        let rendered: toml::Value = toml::from_str(&rendered).unwrap();
        assert_eq!(rendered["bmc_request_concurrency"].as_integer(), Some(1));
        let collectors = &rendered["collectors"];
        assert_eq!(
            collectors["sensors"]["sensor_fetch_interval"].as_str(),
            Some("120s")
        );
        assert_eq!(
            collectors["discovery"]["refresh_interval"].as_str(),
            Some("86400s")
        );
        assert_eq!(collectors["reachability"]["enabled"].as_bool(), Some(false));
        assert_eq!(collectors["reachability"]["interval"].as_str(), Some("90s"));
        assert_eq!(collectors["logs"]["mode"].as_str(), Some("sse"));
        assert_eq!(
            collectors["logs"]["auto"]["retry_sse_after_downgrade"].as_bool(),
            Some(false)
        );
    }

    #[test]
    fn renders_complete_collection_configuration_at_native_table_boundaries() {
        let settings = include_str!("../../tests/fixtures/nico-health-settings.toml");
        let settings = settings.replace("\n[", "\n[nico_health.");
        let collector = parse_test_config(
            &format!("[nico_health]\n{settings}\n[[targets]]\nip = '192.0.2.1'\nmac = '00:11:22:33:44:55'\ncredentials_profile = 'bmc'"),
            "test.toml",
        ).unwrap();
        let native = render_nico_config(&collector, &NicoConfigOptions::default()).unwrap();
        let native: toml::Value = toml::from_str(&native).unwrap();
        assert_eq!(native["endpoint_discovery_interval"].as_str(), Some("240s"));
        assert_eq!(native["bmc_request_concurrency"].as_integer(), Some(2));
        assert_eq!(
            native["rate_limit"]["bucket_replenish"].as_str(),
            Some("100ms")
        );
        assert_eq!(
            native["collectors"]["leak_detector"]["enabled"].as_bool(),
            Some(false)
        );
        assert_eq!(
            native["collectors"]["logs"]["auto"]["logs_collection_interval"].as_str(),
            Some("600s")
        );
        assert!(
            native["endpoint_sources"]
                .get("endpoint_discovery_interval")
                .is_none()
        );
    }

    #[test]
    fn renders_optional_collection_settings_independently() {
        for (interval, concurrency) in [
            (Some(120), Some(2)),
            (Some(300), None),
            (None, Some(1)),
            (None, None),
            (Some(u32::MAX), Some(4)),
        ] {
            let mut settings = String::from("[nico_health]\n");
            if let Some(interval) = interval {
                settings.push_str(&format!("sensor_fetch_interval_seconds = {interval}\n"));
            }
            if let Some(concurrency) = concurrency {
                settings.push_str(&format!("bmc_request_concurrency = {concurrency}\n"));
            }
            let collector = parse_test_config(
                &format!(
                    "{settings}\n[[targets]]\nip = '192.0.2.1'\nmac = '00:11:22:33:44:55'\ncredentials_profile = 'bmc'"
                ),
                "test.toml",
            ).expect("collector config");
            let rendered =
                render_nico_config(&collector, &NicoConfigOptions::default()).expect("NICo config");
            let rendered: toml::Value = toml::from_str(&rendered).expect("rendered TOML");
            assert_eq!(
                rendered
                    .get("bmc_request_concurrency")
                    .and_then(toml::Value::as_integer),
                Some(i64::from(concurrency.unwrap_or(2))),
            );
            let sensors = &rendered["collectors"]["sensors"];
            assert_eq!(sensors["enabled"].as_bool(), Some(true));
            assert_eq!(
                sensors["sensor_fetch_interval"].as_str(),
                Some(format!("{}s", interval.unwrap_or(60)).as_str())
            );
            assert!(sensors.get("include_sensor_thresholds").is_none());
            assert_eq!(
                rendered["collectors"]["logs"]["mode"].as_str(),
                Some("auto")
            );
            assert_eq!(rendered["attributes"]["gpu_identity"].as_bool(), Some(true));
        }
    }

    #[test]
    fn rejects_invalid_collection_settings_without_echoing_input() {
        for setting in ["sensor_fetch_interval_seconds", "bmc_request_concurrency"] {
            for value in ["0", "-1", "1.5", "4294967296", "true", "'sensitive-value'"] {
                let input = format!(
                    "[nico_health]\n{setting} = {value}\n[credentials]\nfile = 'unused.json'"
                );
                let error = parse_config(&input, "test.toml").unwrap_err().to_string();
                assert!(
                    error.contains("invalid TOML config"),
                    "{setting} = {value}: {error}"
                );
                assert!(!error.contains("sensitive-value"));
            }
        }
        let error = parse_config(
            "[nico_health]\nsensor_fetch_intervl_seconds = 120\n[credentials]\nfile = 'unused.json'",
            "test.toml",
        )
        .unwrap_err()
        .to_string();
        assert!(error.contains("invalid TOML config"));
    }

    #[test]
    fn renders_all_target_kinds_as_generic_static_endpoints() {
        let collector = parse_test_config(
            r#"
            [[targets]]
            credentials_profile = "bmc"
            kind = "unknown"
            ip = "10.0.0.1"
            mac = "02:00:0a:00:00:01"

            [[targets]]
            credentials_profile = "bmc"
            kind = "compute"
            ip = "10.0.0.2"
            mac = "02:00:0a:00:00:02"

            [[targets]]
            credentials_profile = "bmc"
            kind = "nvswitch"
            ip = "10.0.0.3"
            mac = "02:00:0a:00:00:03"

            [[targets]]
            credentials_profile = "bmc"
            kind = "power_shelf"
            ip = "10.0.0.4"
            mac = "02:00:0a:00:00:04"
            "#,
            "test.toml",
        )
        .expect("collector config");
        let rendered =
            render_nico_config(&collector, &NicoConfigOptions::default()).expect("NICo config");
        let rendered: toml::Value = toml::from_str(&rendered).expect("rendered TOML");
        let endpoints = rendered["endpoint_sources"]["static_bmc_endpoints"]
            .as_array()
            .expect("static endpoints");

        assert_eq!(endpoints.len(), 4);
        for endpoint in endpoints {
            for descriptor in ["unknown", "machine", "switch", "power_shelf"] {
                assert!(
                    endpoint.get(descriptor).is_none(),
                    "unexpected {descriptor}"
                );
            }
        }
    }

    #[test]
    fn rendered_nico_config_enables_logs_collector() {
        let collector = parse_test_config(
            r#"
            [[targets]]
            credentials_profile = "bmc"
            kind = "compute"
            ip = "10.0.0.1"
            mac = "02:00:0a:00:00:01"
            "#,
            "test.toml",
        )
        .expect("collector config");
        let rendered =
            render_nico_config(&collector, &NicoConfigOptions::default()).expect("NICo config");
        let rendered: toml::Value = toml::from_str(&rendered).expect("rendered TOML");

        assert_eq!(
            rendered["collectors"]["logs"]["enabled"].as_bool(),
            Some(true)
        );
        assert_eq!(
            rendered["collectors"]["logs"]["mode"].as_str(),
            Some("auto")
        );
        assert_eq!(
            rendered["collectors"]["logs"]["auto"]["retry_sse_after_downgrade"].as_bool(),
            Some(true)
        );
        assert_eq!(
            rendered["collectors"]["firmware"]["enabled"].as_bool(),
            Some(true)
        );
        assert_eq!(
            rendered["collectors"]["firmware"]["firmware_refresh_interval"].as_str(),
            Some("1800s")
        );
        assert_eq!(
            rendered["processors"]["resource_status"]["enabled"].as_bool(),
            Some(true)
        );
    }

    #[test]
    fn rendered_nico_config_can_disable_resource_status_collection() {
        let collector = parse_test_config(
            r#"
            [nico_health]
            firmware_enabled = true
            firmware_refresh_interval_seconds = 7200

            [[targets]]
            credentials_profile = "bmc"
            kind = "compute"
            ip = "10.0.0.1"
            mac = "02:00:0a:00:00:01"
            "#,
            "test.toml",
        )
        .expect("collector config");
        let rendered = render_nico_config(
            &collector,
            &NicoConfigOptions {
                resource_status_collection: false,
                resource_status_firmware_poll_interval_seconds: 60,
                ..NicoConfigOptions::default()
            },
        )
        .expect("NICo config");
        let rendered: toml::Value = toml::from_str(&rendered).expect("rendered TOML");

        assert_eq!(
            rendered["collectors"]["firmware"]["enabled"].as_bool(),
            Some(true)
        );
        assert_eq!(
            rendered["collectors"]["firmware"]["firmware_refresh_interval"].as_str(),
            Some("7200s")
        );
        assert_eq!(
            rendered["processors"]["resource_status"]["enabled"].as_bool(),
            Some(false)
        );
    }

    #[test]
    fn rendered_nico_config_rejects_zero_resource_status_interval() {
        let collector = parse_test_config(
            r#"
            [[targets]]
            credentials_profile = "bmc"
            kind = "compute"
            ip = "10.0.0.1"
            mac = "02:00:0a:00:00:01"
            "#,
            "test.toml",
        )
        .expect("collector config");

        let error = render_nico_config(
            &collector,
            &NicoConfigOptions {
                resource_status_firmware_poll_interval_seconds: 0,
                ..NicoConfigOptions::default()
            },
        )
        .expect_err("zero interval must fail");

        assert_eq!(
            error.to_string(),
            "NICo resource status firmware poll interval must be greater than zero"
        );
    }

    #[test]
    fn rendered_nico_config_always_enables_gpu_identity() {
        let collector = parse_test_config(
            r#"
            [[targets]]
            credentials_profile = "bmc"
            kind = "compute"
            ip = "10.0.0.1"
            mac = "02:00:0a:00:00:01"
            "#,
            "test.toml",
        )
        .expect("collector config");
        let rendered =
            render_nico_config(&collector, &NicoConfigOptions::default()).expect("NICo config");
        let rendered: toml::Value = toml::from_str(&rendered).expect("rendered TOML");

        assert_eq!(rendered["attributes"]["gpu_identity"].as_bool(), Some(true));
    }

    #[test]
    fn rendered_nico_config_enables_tracing_sink() {
        let collector = parse_test_config(
            r#"
            [[targets]]
            credentials_profile = "bmc"
            kind = "compute"
            ip = "10.0.0.1"
            mac = "02:00:0a:00:00:01"
            "#,
            "test.toml",
        )
        .expect("collector config");
        let rendered =
            render_nico_config(&collector, &NicoConfigOptions::default()).expect("NICo config");
        let rendered: toml::Value = toml::from_str(&rendered).expect("rendered TOML");

        assert_eq!(
            rendered["sinks"]["tracing"]["enabled"].as_bool(),
            Some(true)
        );
    }

    #[test]
    fn rendered_nico_config_trims_sink_options() {
        let collector = parse_test_config(
            r#"
            [[targets]]
            credentials_profile = "bmc"
            kind = "compute"
            ip = "10.0.0.1"
            mac = "02:00:0a:00:00:01"
            "#,
            "test.toml",
        )
        .expect("collector config");
        let rendered = render_nico_config(
            &collector,
            &NicoConfigOptions {
                otlp_endpoint: "  http://localhost:4317  ".to_string(),
                metrics_endpoint: "  0.0.0.0:9009  ".to_string(),
                metrics_prefix: "  fleetint  ".to_string(),
                ..NicoConfigOptions::default()
            },
        )
        .expect("NICo config");
        let rendered: toml::Value = toml::from_str(&rendered).expect("rendered TOML");

        assert_eq!(
            rendered["sinks"]["otlp"]["targets"][0]["endpoint"].as_str(),
            Some("http://localhost:4317")
        );
        assert_eq!(
            rendered["metrics"]["endpoint"].as_str(),
            Some("0.0.0.0:9009")
        );
        assert_eq!(rendered["metrics"]["prefix"].as_str(), Some("fleetint"));
    }

    #[test]
    fn rendered_nico_config_references_credentials_without_inlining() {
        let collector = parse_test_config(
            r#"
            [[targets]]
            credentials_profile = "bmc"
            kind = "compute"
            ip = "2001:db8::1"
            mac = "00:11:22:33:44:55"
            "#,
            "test.toml",
        )
        .expect("collector config");
        let rendered =
            render_nico_config(&collector, &NicoConfigOptions::default()).expect("NICo config");
        assert!(!rendered.contains("example-user"));
        assert!(!rendered.contains("example-password"));
        let rendered: toml::Value = toml::from_str(&rendered).expect("rendered TOML");
        let endpoint = &rendered["endpoint_sources"]["static_bmc_endpoints"][0];

        assert_eq!(
            rendered["endpoint_sources"]["static_credentials_file"].as_str(),
            collector.credentials_file.to_str()
        );
        assert_eq!(
            rendered["endpoint_sources"]["nico_api"]["enabled"].as_bool(),
            Some(false)
        );
        assert!(endpoint.get("username").is_none());
        assert!(endpoint.get("password").is_none());
        assert_eq!(endpoint["credentials_profile"].as_str(), Some("bmc"));
        assert_eq!(endpoint["labels"]["oob"].as_str(), Some("true"));
        assert!(endpoint["labels"].get("machine_name").is_none());
        assert!(endpoint.get("port").is_none());
    }

    #[test]
    fn merged_nico_config_preserves_customer_sources_plugins_and_sinks() {
        let collector = parse_test_config(
            r#"
            compute_zone = "us-west-dc1"
            node_group = "gb200-training"

            [[targets]]
            credentials_profile = "bmc"
            kind = "compute"
            name = "node-01"
            ip = "10.0.0.1"
            mac = "AA:BB:CC:DD:EE:FF"
            "#,
            "test.toml",
        )
        .unwrap();
        let base = r#"
customer_root_setting = "preserved"

[endpoint_sources]
identity = ["mac", "ip"]
static_credentials_file = "/gmi/credentials.json"

[endpoint_sources.sql]
database_url_file = "/gmi/database-url"
credentials_profile = "gmi-bmc"
query = "SELECT bmc_ip, bmc_mac, machine_name FROM inventory"

[collectors.custom_plugin]
enabled = true
plugin_option = "preserved"

[collectors.logs]
enabled = false
customer_option = "preserved"

[processors.customer]
enabled = true

[attributes]
customer_attribute = true

[sinks.customer]
enabled = true

[sinks.otlp]
enabled = true

[[sinks.otlp.targets]]
endpoint = "https://customer-otel.example:4317"
batch_size = 7
"#;
        let merged = merge_nico_config(base, &collector, &NicoConfigOptions::default()).unwrap();
        let merged: Value = toml::from_str(&merged).unwrap();

        assert_eq!(merged["customer_root_setting"].as_str(), Some("preserved"));
        assert_eq!(
            merged["endpoint_sources"]["sql"]["query"].as_str(),
            Some("SELECT bmc_ip, bmc_mac, machine_name FROM inventory")
        );
        assert!(
            merged["endpoint_sources"]
                .get("static_bmc_endpoints")
                .is_none()
        );
        assert_eq!(
            merged["collectors"]["custom_plugin"]["plugin_option"].as_str(),
            Some("preserved")
        );
        assert_eq!(
            merged["collectors"]["logs"]["customer_option"].as_str(),
            Some("preserved")
        );
        assert_eq!(
            merged["collectors"]["logs"]["enabled"].as_bool(),
            Some(true)
        );
        assert_eq!(
            merged["processors"]["customer"]["enabled"].as_bool(),
            Some(true)
        );
        assert_eq!(
            merged["attributes"]["customer_attribute"].as_bool(),
            Some(true)
        );
        assert_eq!(merged["attributes"]["gpu_identity"].as_bool(), Some(true));
        assert_eq!(merged["sinks"]["customer"]["enabled"].as_bool(), Some(true));
        let targets = merged["sinks"]["otlp"]["targets"].as_array().unwrap();
        assert_eq!(targets.len(), 2);
        assert_eq!(
            targets[0]["endpoint"].as_str(),
            Some("https://customer-otel.example:4317")
        );
        assert_eq!(
            targets[1]["endpoint"].as_str(),
            Some("http://localhost:4317")
        );
        assert_eq!(targets[1]["include_diagnostics"].as_bool(), Some(true));
        assert_eq!(targets[1]["include_alert_details"].as_bool(), Some(true));
    }

    #[test]
    fn merged_nico_config_enriches_matching_static_endpoint_without_replacing_it() {
        let collector = parse_test_config(
            r#"
            compute_zone = "us-west-dc1"
            node_group = "gb200-training"

            [[targets]]
            credentials_profile = "bmc"
            name = "node-01"
            ip = "10.0.0.99"
            mac = "AA:BB:CC:DD:EE:FF"
            labels = { fi_custom = "fi-value" }
            "#,
            "test.toml",
        )
        .unwrap();
        let base = r#"
[endpoint_sources]
static_credentials_file = "/gmi/credentials.json"

[[endpoint_sources.static_bmc_endpoints]]
ip = "10.0.0.1"
port = 8443
mac = "aa-bb-cc-dd-ee-ff"
credentials_profile = "gmi-profile"
labels = { customer = "preserved", machine_name = "old-name" }

[collectors.custom]
enabled = true

[sinks.otlp]
enabled = true

[[sinks.otlp.targets]]
endpoint = "http://localhost:4317"
customer_tls_option = "preserved"
"#;
        let merged = merge_nico_config(base, &collector, &NicoConfigOptions::default()).unwrap();
        let merged: Value = toml::from_str(&merged).unwrap();
        let endpoint = &merged["endpoint_sources"]["static_bmc_endpoints"][0];

        assert_eq!(endpoint["ip"].as_str(), Some("10.0.0.1"));
        assert_eq!(endpoint["port"].as_integer(), Some(8443));
        assert_eq!(
            endpoint["credentials_profile"].as_str(),
            Some("gmi-profile")
        );
        assert_eq!(endpoint["labels"]["customer"].as_str(), Some("preserved"));
        assert_eq!(endpoint["labels"]["machine_name"].as_str(), Some("node-01"));
        assert_eq!(
            endpoint["labels"]["compute_zone"].as_str(),
            Some("us-west-dc1")
        );
        assert_eq!(endpoint["labels"]["oob"].as_str(), Some("true"));
        assert_eq!(endpoint["labels"]["fi_custom"].as_str(), Some("fi-value"));
        let targets = merged["sinks"]["otlp"]["targets"].as_array().unwrap();
        assert_eq!(targets.len(), 1);
        assert_eq!(
            targets[0]["customer_tls_option"].as_str(),
            Some("preserved")
        );
        assert_eq!(targets[0]["batch_size"].as_integer(), Some(512));
    }

    #[test]
    fn merged_nico_config_does_not_echo_invalid_customer_content() {
        let collector = parse_test_config(
            r#"
            [[targets]]
            credentials_profile = "bmc"
            ip = "10.0.0.1"
            mac = "AA:BB:CC:DD:EE:FF"
            "#,
            "test.toml",
        )
        .unwrap();
        let error = merge_nico_config(
            "password = 'sensitive-value'\ninvalid = [",
            &collector,
            &NicoConfigOptions::default(),
        )
        .unwrap_err()
        .to_string();

        assert_eq!(error, "existing NICo configuration is not valid TOML");
        assert!(!error.contains("sensitive-value"));
    }
}
