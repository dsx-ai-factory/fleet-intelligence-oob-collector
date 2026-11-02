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

use serde::Serialize;

use super::CollectorConfig;

#[derive(Clone, Debug)]
pub struct NicoConfigOptions {
    pub otlp_endpoint: String,
    pub metrics_endpoint: String,
    pub metrics_prefix: String,
}

impl Default for NicoConfigOptions {
    fn default() -> Self {
        Self {
            otlp_endpoint: "http://localhost:4317".to_string(),
            metrics_endpoint: "0.0.0.0:9009".to_string(),
            metrics_prefix: "fleetint".to_string(),
        }
    }
}

/// Render shared targets and explicit NICo overrides without embedding credentials.
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

    let nico_config = NicoConfig {
        bmc_request_concurrency: config.nico_health.bmc_request_concurrency,
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
        collectors: config.nico_health.native_collectors(),
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
    /// Ignored by NICo builds that predate the flag, which do not reject
    /// unknown OTLP target keys.
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
        assert!(rendered.get("bmc_request_concurrency").is_none());
        assert!(rendered["collectors"].get("sensors").is_none());
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
                concurrency.map(i64::from),
            );
            let sensors = rendered["collectors"].get("sensors");
            if let Some(interval) = interval {
                let sensors = sensors.expect("sensor override");
                assert_eq!(sensors["enabled"].as_bool(), Some(true));
                assert_eq!(
                    sensors["sensor_fetch_interval"].as_str(),
                    Some(format!("{interval}s").as_str())
                );
                assert!(sensors.get("include_sensor_thresholds").is_none());
            } else {
                assert!(sensors.is_none());
            }
            assert_eq!(rendered["collectors"]["logs"]["mode"].as_str(), Some("sse"));
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
        assert_eq!(rendered["collectors"]["logs"]["mode"].as_str(), Some("sse"));
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
}
