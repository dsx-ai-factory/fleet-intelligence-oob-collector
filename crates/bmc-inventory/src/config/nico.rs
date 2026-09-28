use std::collections::BTreeMap;
use std::error::Error;
use std::net::IpAddr;

use serde::Serialize;

use super::{CollectorConfig, TargetKind};

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

pub fn render_nico_config(
    config: &CollectorConfig,
    options: &NicoConfigOptions,
) -> Result<String, Box<dyn Error>> {
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
            mac: &target.mac,
            username: &target.username,
            password: &target.password,
            labels: target.labels.clone(),
            unknown: (target.kind == TargetKind::Unknown).then_some(NicoEndpointKind {}),
            machine: (target.kind == TargetKind::Compute).then_some(NicoEndpointKind {}),
            switch: (target.kind == TargetKind::NvSwitch).then_some(NicoEndpointKind {}),
            power_shelf: (target.kind == TargetKind::PowerShelf).then_some(NicoEndpointKind {}),
        })
        .collect();

    let nico_config = NicoConfig {
        endpoint_sources: NicoEndpointSources {
            carbide_api: NicoEnabled { enabled: false },
            static_bmc_endpoints,
        },
        collectors: NicoCollectors {
            logs: NicoLogsCollector {
                enabled: true,
                // "sse" streams BMC log entries over Redfish SSE without the
                // LogService polling fallback "auto" applies per endpoint, so an
                // endpoint that lacks /EventService emits no log records.
                mode: "sse",
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

#[derive(Serialize)]
struct NicoConfig<'a> {
    endpoint_sources: NicoEndpointSources<'a>,
    collectors: NicoCollectors,
    attributes: NicoAttributes,
    sinks: NicoSinks<'a>,
    metrics: NicoMetrics<'a>,
}

#[derive(Serialize)]
struct NicoEndpointSources<'a> {
    carbide_api: NicoEnabled,
    static_bmc_endpoints: Vec<NicoStaticBmcEndpoint<'a>>,
}

#[derive(Serialize)]
struct NicoStaticBmcEndpoint<'a> {
    ip: IpAddr,
    #[serde(skip_serializing_if = "Option::is_none")]
    port: Option<u16>,
    mac: &'a str,
    username: &'a str,
    password: &'a str,
    labels: BTreeMap<String, String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    unknown: Option<NicoEndpointKind>,
    #[serde(skip_serializing_if = "Option::is_none")]
    machine: Option<NicoEndpointKind>,
    #[serde(skip_serializing_if = "Option::is_none")]
    switch: Option<NicoEndpointKind>,
    #[serde(skip_serializing_if = "Option::is_none")]
    power_shelf: Option<NicoEndpointKind>,
}

#[derive(Serialize)]
struct NicoEndpointKind {}

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
struct NicoCollectors {
    logs: NicoLogsCollector,
}

/// NICo leaves the logs collector disabled unless the section is present, so it
/// must be rendered explicitly for BMC log entries to reach the OTLP sink.
#[derive(Serialize)]
struct NicoLogsCollector {
    enabled: bool,
    mode: &'static str,
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

    #[test]
    fn renders_nico_static_config_from_collector_config() {
        let collector = parse_config(
            r#"
            compute_zone = "us-west-dc1"
            node_group = "gb200-training"

            [[machines]]
            name = "node-01"
            ip = "10.0.0.1"
            port = 8443
            username = "admin"
            password = "secret"
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
        assert_eq!(endpoint["mac"].as_str(), Some("02:00:0a:00:00:01"));
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
        assert!(endpoint.get("machine").is_some());
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
    }

    #[test]
    fn renders_nico_endpoint_discriminator_for_each_target_kind() {
        let collector = parse_config(
            r#"
            [[targets]]
            kind = "unknown"
            ip = "10.0.0.1"
            username = "admin"
            password = "secret"

            [[targets]]
            kind = "compute"
            ip = "10.0.0.2"
            username = "admin"
            password = "secret"

            [[targets]]
            kind = "nvswitch"
            ip = "10.0.0.3"
            username = "admin"
            password = "secret"

            [[targets]]
            kind = "power_shelf"
            ip = "10.0.0.4"
            username = "admin"
            password = "secret"
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

        assert!(endpoints[0].get("unknown").is_some());
        assert!(endpoints[0].get("machine").is_none());
        assert!(endpoints[0].get("switch").is_none());
        assert!(endpoints[0].get("power_shelf").is_none());
        assert!(endpoints[1].get("unknown").is_none());
        assert!(endpoints[1].get("machine").is_some());
        assert!(endpoints[2].get("switch").is_some());
        assert!(endpoints[3].get("power_shelf").is_some());
    }

    #[test]
    fn rendered_nico_config_enables_logs_collector() {
        let collector = parse_config(
            r#"
            [[machines]]
            ip = "10.0.0.1"
            username = "admin"
            password = "secret"
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
        let collector = parse_config(
            r#"
            [[machines]]
            ip = "10.0.0.1"
            username = "admin"
            password = "secret"
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
        let collector = parse_config(
            r#"
            [[machines]]
            ip = "10.0.0.1"
            username = "admin"
            password = "secret"
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
        let collector = parse_config(
            r#"
            [[machines]]
            ip = "10.0.0.1"
            username = "admin"
            password = "secret"
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
    fn rendered_nico_config_escapes_credentials_and_omits_empty_labels() {
        let collector = parse_config(
            r#"
            [[machines]]
            ip = "2001:db8::1"
            mac = "00:11:22:33:44:55"
            username = 'admin"name'
            password = 'secret"value'
            "#,
            "test.toml",
        )
        .expect("collector config");
        let rendered =
            render_nico_config(&collector, &NicoConfigOptions::default()).expect("NICo config");
        let rendered: toml::Value = toml::from_str(&rendered).expect("rendered TOML");
        let endpoint = &rendered["endpoint_sources"]["static_bmc_endpoints"][0];

        assert_eq!(endpoint["username"].as_str(), Some("admin\"name"));
        assert_eq!(endpoint["password"].as_str(), Some("secret\"value"));
        assert_eq!(endpoint["labels"]["oob"].as_str(), Some("true"));
        assert!(endpoint["labels"].get("machine_name").is_none());
        assert!(endpoint.get("port").is_none());
    }
}
