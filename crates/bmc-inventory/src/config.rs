use std::collections::{BTreeMap, HashSet};
use std::error::Error;
use std::fmt;
use std::net::IpAddr;

use serde::{Deserialize, Serialize};

const MAX_PLACEMENT_NAME_BYTES: usize = 255;
const MAX_NICO_LABELS: usize = 32;
const MAX_NICO_LABEL_VALUE_BYTES: usize = 1024;
const RESERVED_NICO_LABELS: &[&str] = &[
    "collector_type",
    "endpoint_ip",
    "endpoint_key",
    "endpoint_mac",
    "machine_id",
    "machine_slot_number",
    "machine_tray_index",
    "nvlink_domain_uuid",
    "rack_id",
    "serial_number",
    "switch_id",
    "switch_slot_number",
    "switch_tray_index",
    "system_uuid",
];

#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct Placement {
    pub compute_zone: String,
    pub node_group: String,
}

#[derive(Clone, Debug)]
pub struct CollectorConfig {
    pub placement: Placement,
    pub targets: Vec<Target>,
}

#[derive(Clone)]
pub struct Target {
    pub ip: IpAddr,
    pub port: Option<u16>,
    pub mac: String,
    pub address: String,
    pub username: String,
    pub password: String,
    pub labels: BTreeMap<String, String>,
}

impl fmt::Debug for Target {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("Target")
            .field("mac", &self.mac)
            .field("address", &self.address)
            .field("username", &self.username)
            .field("labels", &self.labels)
            .field("password", &"[REDACTED]")
            .finish()
    }
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Config {
    #[serde(default)]
    compute_zone: String,
    #[serde(default)]
    node_group: String,
    #[serde(default)]
    machines: Vec<MachineConfig>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct MachineConfig {
    ip: IpAddr,
    #[serde(default)]
    port: Option<u16>,
    #[serde(default)]
    mac: Option<String>,
    username: String,
    password: String,
    #[serde(default)]
    labels: BTreeMap<String, String>,
}

pub fn load_config(path: &str) -> Result<CollectorConfig, Box<dyn Error>> {
    let contents = std::fs::read_to_string(path)?;
    parse_config(&contents, path)
}

pub fn load_targets(path: &str) -> Result<Vec<Target>, Box<dyn Error>> {
    Ok(load_config(path)?.targets)
}

fn parse_config(contents: &str, path: &str) -> Result<CollectorConfig, Box<dyn Error>> {
    let config: Config = toml::from_str(contents)
        .map_err(|error| format!("failed to parse TOML config {path}: {error}"))?;

    if config.machines.is_empty() {
        return Err(format!("no BMC targets found in {path}; configure [[machines]]").into());
    }

    let placement = Placement {
        compute_zone: normalize_placement_name("compute_zone", config.compute_zone)?,
        node_group: normalize_placement_name("node_group", config.node_group)?,
    };

    let mut macs = HashSet::new();
    let mut targets = Vec::with_capacity(config.machines.len());
    for (index, machine) in config.machines.into_iter().enumerate() {
        let target = machine.into_target(index)?;
        validate_nico_labels(index, &target.labels, &placement)?;
        if !macs.insert(target.mac.to_ascii_lowercase()) {
            return Err(format!("duplicate BMC MAC {:?}", target.mac).into());
        }
        targets.push(target);
    }

    Ok(CollectorConfig { placement, targets })
}

fn validate_nico_labels(
    index: usize,
    labels: &BTreeMap<String, String>,
    placement: &Placement,
) -> Result<(), Box<dyn Error>> {
    if labels.contains_key("compute_zone") || labels.contains_key("node_group") {
        return Err(format!(
            "machines[{index}].labels must not define compute_zone or node_group; use the top-level placement fields"
        )
        .into());
    }
    let placement_label_count = usize::from(!placement.compute_zone.is_empty())
        + usize::from(!placement.node_group.is_empty());
    if labels.len() + placement_label_count > MAX_NICO_LABELS {
        return Err(format!(
            "machines[{index}].labels and placement support at most {MAX_NICO_LABELS} NICo labels"
        )
        .into());
    }
    for (name, value) in labels {
        let mut chars = name.chars();
        let valid_start = chars
            .next()
            .is_some_and(|character| character.is_ascii_alphabetic() || character == '_');
        let valid_rest =
            chars.all(|character| character.is_ascii_alphanumeric() || character == '_');
        if !valid_start || !valid_rest {
            return Err(format!(
                "machines[{index}].labels key {name:?} must match [a-zA-Z_][a-zA-Z0-9_]*"
            )
            .into());
        }
        if RESERVED_NICO_LABELS.contains(&name.as_str()) {
            return Err(
                format!("machines[{index}].labels key {name:?} is reserved by NICo").into(),
            );
        }
        if value.len() > MAX_NICO_LABEL_VALUE_BYTES {
            return Err(format!(
                "machines[{index}].labels value for {name:?} exceeds {MAX_NICO_LABEL_VALUE_BYTES} bytes"
            )
            .into());
        }
    }
    Ok(())
}

fn normalize_placement_name(name: &str, value: String) -> Result<String, Box<dyn Error>> {
    let value = value.trim().to_string();
    if value.len() > MAX_PLACEMENT_NAME_BYTES {
        return Err(format!("{name} must not exceed {MAX_PLACEMENT_NAME_BYTES} bytes").into());
    }
    Ok(value)
}

impl MachineConfig {
    fn into_target(self, index: usize) -> Result<Target, Box<dyn Error>> {
        let context = format!("machines[{index}]");
        let mac = match self.mac {
            Some(mac) if mac.trim().is_empty() => {
                return Err(format!("{context}.mac cannot be blank").into());
            }
            Some(mac) => mac,
            None => derive_mac_from_ip(self.ip),
        };
        if self.username.trim().is_empty() {
            return Err(format!("{context}.username is required").into());
        }
        if self.password.trim().is_empty() {
            return Err(format!("{context}.password is required by the OOB collector").into());
        }
        let address = match (self.ip, self.port) {
            (IpAddr::V4(ip), Some(port)) => format!("{ip}:{port}"),
            (IpAddr::V6(ip), Some(port)) => format!("[{ip}]:{port}"),
            (IpAddr::V4(ip), None) => ip.to_string(),
            (IpAddr::V6(ip), None) => format!("[{ip}]"),
        };

        Ok(Target {
            ip: self.ip,
            port: self.port,
            mac,
            address,
            username: self.username,
            password: self.password,
            labels: self.labels,
        })
    }
}

fn derive_mac_from_ip(ip: IpAddr) -> String {
    let ip_bytes = match ip {
        IpAddr::V4(ip) => ip.octets(),
        IpAddr::V6(ip) => {
            let octets = ip.octets();
            [octets[12], octets[13], octets[14], octets[15]]
        }
    };
    format!(
        "02:00:{:02x}:{:02x}:{:02x}:{:02x}",
        ip_bytes[0], ip_bytes[1], ip_bytes[2], ip_bytes[3]
    )
}

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
        .map(|target| {
            let mut labels = target.labels.clone();
            if !config.placement.compute_zone.is_empty() {
                labels.insert(
                    "compute_zone".to_string(),
                    config.placement.compute_zone.clone(),
                );
            }
            if !config.placement.node_group.is_empty() {
                labels.insert(
                    "node_group".to_string(),
                    config.placement.node_group.clone(),
                );
            }
            NicoStaticBmcEndpoint {
                ip: target.ip,
                port: target.port,
                mac: &target.mac,
                username: &target.username,
                password: &target.password,
                labels,
                machine: NicoMachine {},
            }
        })
        .collect();

    let nico_config = NicoConfig {
        endpoint_sources: NicoEndpointSources {
            carbide_api: NicoEnabled { enabled: false },
            static_bmc_endpoints,
        },
        sinks: NicoSinks {
            health_report: NicoEnabled { enabled: false },
            rack_health_report: NicoEnabled { enabled: false },
            switch_health_report: NicoEnabled { enabled: false },
            power_shelf_health_report: NicoEnabled { enabled: false },
            prometheus: NicoEnabled { enabled: true },
            otlp: NicoOtlpSink {
                enabled: true,
                endpoint: otlp_endpoint,
                include_diagnostics: false,
                batch_size: 512,
                flush_interval: "2s",
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
    machine: NicoMachine,
}

#[derive(Serialize)]
struct NicoMachine {}

#[derive(Serialize)]
struct NicoSinks<'a> {
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
    endpoint: &'a str,
    include_diagnostics: bool,
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
    use super::*;

    #[test]
    fn loads_top_level_placement_and_machines() {
        let path = write_config(
            "valid",
            r#"
            compute_zone = "  az51 "
            node_group = " dev3-dh1  "

            [[machines]]
            ip = "10.0.0.1"
            port = 8443
            mac = "00:11:22:33:44:55"
            username = "admin"
            password = "secret"
            labels = { site = "dev" }

            [[machines]]
            ip = "2001:db8::1"
            mac = "00:11:22:33:44:66"
            username = "root"
            password = "pass"
            "#,
        );

        let config = load_config(path.to_str().expect("path")).expect("config");

        assert_eq!(
            config.placement,
            Placement {
                compute_zone: "az51".to_string(),
                node_group: "dev3-dh1".to_string(),
            }
        );
        assert_eq!(config.targets.len(), 2);
        assert_eq!(config.targets[0].mac, "00:11:22:33:44:55");
        assert_eq!(config.targets[0].address, "10.0.0.1:8443");
        assert_eq!(config.targets[1].mac, "00:11:22:33:44:66");
        assert_eq!(config.targets[1].address, "[2001:db8::1]");
    }

    #[test]
    fn placement_defaults_to_unassigned() {
        let path = write_config(
            "unassigned",
            r#"
            [[machines]]
            ip = "10.0.0.1"
            mac = "00:11:22:33:44:55"
            username = "admin"
            password = "secret"
            labels = { site = "dev" }
            "#,
        );

        let config = load_config(path.to_str().expect("path")).expect("config");

        assert_eq!(config.placement, Placement::default());
    }

    #[test]
    fn derives_mac_from_last_four_ip_bytes_when_omitted() {
        let path = write_config(
            "derived-mac",
            r#"
            [[machines]]
            ip = "10.0.0.10"
            username = "admin"
            password = "secret"

            [[machines]]
            ip = "2001:db8::102:304"
            username = "admin"
            password = "secret"
            "#,
        );

        let config = load_config(path.to_str().expect("path")).expect("config");

        assert_eq!(config.targets[0].mac, "02:00:0a:00:00:0a");
        assert_eq!(config.targets[1].mac, "02:00:01:02:03:04");
    }

    #[test]
    fn explicit_mac_takes_precedence_and_blank_mac_is_rejected() {
        let explicit = parse_config(
            r#"
            [[machines]]
            ip = "10.0.0.10"
            mac = "aa:bb:cc:dd:ee:ff"
            username = "admin"
            password = "secret"
            "#,
            "test.toml",
        )
        .expect("config");
        assert_eq!(explicit.targets[0].mac, "aa:bb:cc:dd:ee:ff");

        let error = parse_config(
            r#"
            [[machines]]
            ip = "10.0.0.10"
            mac = " "
            username = "admin"
            password = "secret"
            "#,
            "test.toml",
        )
        .expect_err("blank MAC should fail")
        .to_string();
        assert!(error.contains("mac cannot be blank"));
    }

    #[test]
    fn repository_oob_config_example_is_valid() {
        let config = parse_config(
            include_str!("../../../config.example.toml"),
            "config.example.toml",
        )
        .expect("OOB config example");

        assert_eq!(config.targets.len(), 1);
        assert_eq!(config.targets[0].mac, "aa:bb:cc:dd:ee:ff");
        assert_eq!(config.placement.compute_zone, "az51");
        assert_eq!(config.placement.node_group, "dev3-dh1");
    }

    #[test]
    fn rejects_missing_password() {
        let path = write_config(
            "missing-password",
            r#"
            [[machines]]
            ip = "10.0.0.1"
            mac = "00:11:22:33:44:55"
            username = "admin"
            "#,
        );

        let error = load_config(path.to_str().expect("path"))
            .expect_err("missing password should fail")
            .to_string();

        assert!(error.contains("missing field `password`"));
    }

    #[test]
    fn rejects_blank_password() {
        let path = write_config(
            "blank-password",
            r#"
            [[machines]]
            ip = "10.0.0.1"
            mac = "00:11:22:33:44:55"
            username = "admin"
            password = "   "
            "#,
        );

        let error = load_config(path.to_str().expect("path"))
            .expect_err("blank password should fail")
            .to_string();

        assert!(error.contains("password is required by the OOB collector"));
    }

    #[test]
    fn rejects_empty_machine_list() {
        let path = write_config("empty", "machines = []");

        let error = load_config(path.to_str().expect("path"))
            .expect_err("empty config should fail")
            .to_string();

        assert!(error.contains("[[machines]]"));
    }

    #[test]
    fn rejects_duplicate_macs_case_insensitively() {
        let path = write_config(
            "duplicate-mac",
            r#"
            [[machines]]
            ip = "10.0.0.1"
            mac = "AA:BB:CC:DD:EE:FF"
            username = "admin"
            password = "secret"

            [[machines]]
            ip = "10.0.0.2"
            mac = "aa:bb:cc:dd:ee:ff"
            username = "admin"
            password = "secret"
            "#,
        );

        let error = load_config(path.to_str().expect("path"))
            .expect_err("duplicate MAC should fail")
            .to_string();

        assert!(error.contains("duplicate BMC MAC"));
    }

    #[test]
    fn rejects_unknown_fields() {
        let path = write_config(
            "unknown",
            r#"
            unexpected = true

            [[machines]]
            ip = "10.0.0.1"
            mac = "00:11:22:33:44:55"
            username = "admin"
            password = "secret"
            "#,
        );

        let error = load_config(path.to_str().expect("path"))
            .expect_err("unknown field should fail")
            .to_string();

        assert!(error.contains("unknown field"));
    }

    #[test]
    fn rejects_placement_names_over_backend_limit() {
        let path = write_config(
            "long-placement",
            &format!(
                r#"
                compute_zone = "{}"

                [[machines]]
                ip = "10.0.0.1"
                mac = "00:11:22:33:44:55"
                username = "admin"
                password = "secret"
                "#,
                "z".repeat(MAX_PLACEMENT_NAME_BYTES + 1)
            ),
        );

        let error = load_config(path.to_str().expect("path"))
            .expect_err("long placement should fail")
            .to_string();

        assert!(error.contains("compute_zone"));
        assert!(error.contains("255"));
    }

    #[test]
    fn renders_nico_static_config_from_collector_config() {
        let collector = parse_config(
            r#"
            compute_zone = "az51"
            node_group = "dev3-dh1"

            [[machines]]
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
        assert_eq!(endpoint["labels"]["compute_zone"].as_str(), Some("az51"));
        assert_eq!(endpoint["labels"]["node_group"].as_str(), Some("dev3-dh1"));
        assert_eq!(endpoint["labels"]["site"].as_str(), Some("dev"));
        assert!(endpoint.get("machine").is_some());
        assert_eq!(
            rendered["sinks"]["otlp"]["endpoint"].as_str(),
            Some("http://localhost:4317")
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
            rendered["sinks"]["otlp"]["endpoint"].as_str(),
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
        assert!(endpoint["labels"].as_table().expect("labels").is_empty());
        assert!(endpoint.get("port").is_none());
    }

    #[test]
    fn rejects_machine_labels_that_override_placement_or_nico_identity() {
        for label in ["compute_zone", "system_uuid"] {
            let error = parse_config(
                &format!(
                    r#"
                    compute_zone = "az51"

                    [[machines]]
                    ip = "10.0.0.1"
                    mac = "00:11:22:33:44:55"
                    username = "admin"
                    password = "secret"
                    labels = {{ {label} = "override" }}
                    "#
                ),
                "test.toml",
            )
            .expect_err("reserved label should fail")
            .to_string();

            assert!(error.contains(label));
        }
    }

    fn write_config(name: &str, content: &str) -> std::path::PathBuf {
        let mut path = std::env::temp_dir();
        path.push(format!(
            "fleet-intelligence-oob-{name}-{}.toml",
            std::process::id()
        ));
        std::fs::write(&path, unindent(content)).expect("write config");
        path
    }

    fn unindent(content: &str) -> String {
        content
            .lines()
            .map(str::trim)
            .collect::<Vec<_>>()
            .join("\n")
    }
}
