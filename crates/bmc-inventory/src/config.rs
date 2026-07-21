use std::collections::{BTreeMap, HashSet};
use std::error::Error;
use std::fmt;
use std::net::IpAddr;

use serde::Deserialize;

#[derive(Clone)]
pub struct Target {
    pub ip: IpAddr,
    pub mac: String,
    pub address: String,
    pub username: String,
    pub password: String,
    pub rack: Option<String>,
    pub labels: BTreeMap<String, String>,
}

impl fmt::Debug for Target {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("Target")
            .field("mac", &self.mac)
            .field("address", &self.address)
            .field("username", &self.username)
            .field("password", &"[REDACTED]")
            .field("rack", &self.rack)
            .field("labels", &self.labels)
            .finish()
    }
}

#[derive(Deserialize)]
struct Config {
    endpoint_sources: EndpointSources,
}

#[derive(Deserialize)]
struct EndpointSources {
    #[serde(default)]
    static_bmc_endpoints: Vec<StaticBmcEndpoint>,
}

#[derive(Deserialize)]
struct StaticBmcEndpoint {
    ip: IpAddr,
    #[serde(default)]
    port: Option<u16>,
    mac: String,
    username: String,
    password: String,
    rack: Option<String>,
    #[serde(default)]
    labels: BTreeMap<String, String>,
}

pub fn load_targets(path: &str) -> Result<Vec<Target>, Box<dyn Error>> {
    let contents = std::fs::read_to_string(path)?;
    parse_targets(&contents, path)
}

fn parse_targets(contents: &str, path: &str) -> Result<Vec<Target>, Box<dyn Error>> {
    let config: Config = toml::from_str(contents)
        .map_err(|error| format!("failed to parse TOML config {path}: {error}"))?;

    if config.endpoint_sources.static_bmc_endpoints.is_empty() {
        return Err(format!(
            "no BMC targets found in {path}; configure endpoint_sources.static_bmc_endpoints"
        )
        .into());
    }

    let mut macs = HashSet::new();
    let mut targets = Vec::with_capacity(config.endpoint_sources.static_bmc_endpoints.len());
    for (index, endpoint) in config
        .endpoint_sources
        .static_bmc_endpoints
        .into_iter()
        .enumerate()
    {
        let target = endpoint.into_target(index)?;
        if !macs.insert(target.mac.clone()) {
            return Err(format!("duplicate BMC MAC {:?}", target.mac).into());
        }
        targets.push(target);
    }

    Ok(targets)
}

impl StaticBmcEndpoint {
    fn into_target(self, index: usize) -> Result<Target, Box<dyn Error>> {
        let context = format!("endpoint_sources.static_bmc_endpoints[{index}]");
        if self.mac.trim().is_empty() {
            return Err(format!("{context}.mac is required").into());
        }
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
            mac: self.mac,
            address,
            username: self.username,
            password: self.password,
            rack: self.rack.filter(|rack| !rack.trim().is_empty()),
            labels: self.labels,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn loads_static_endpoints_and_ignores_other_settings() {
        let path = write_config(
            "valid",
            r#"
            shard = 0
            endpoint_discovery_interval = "5m"

            [endpoint_sources.carbide_api]
            enabled = false

            [[endpoint_sources.static_bmc_endpoints]]
            ip = "10.0.0.1"
            port = 8443
            mac = "00:11:22:33:44:55"
            username = "admin"
            password = "secret"
            rack = "rack-01"
            labels = { compute_zone = "az51", node_group = "dev3-dh1" }
            [[endpoint_sources.static_bmc_endpoints]]
            ip = "2001:db8::1"
            mac = "00:11:22:33:44:66"
            username = "root"
            password = "pass"

            [sinks.prometheus]
            enabled = true
            "#,
        );

        let targets = load_targets(path.to_str().expect("path")).expect("targets");

        assert_eq!(targets.len(), 2);
        assert_eq!(targets[0].mac, "00:11:22:33:44:55");
        assert_eq!(targets[0].address, "10.0.0.1:8443");
        assert_eq!(targets[0].rack.as_deref(), Some("rack-01"));
        assert_eq!(
            targets[0].labels.get("compute_zone").map(String::as_str),
            Some("az51")
        );
        assert_eq!(
            targets[0].labels.get("node_group").map(String::as_str),
            Some("dev3-dh1")
        );
        assert_eq!(targets[1].mac, "00:11:22:33:44:66");
        assert_eq!(targets[1].address, "[2001:db8::1]");
    }

    #[test]
    fn repository_shared_config_example_is_valid() {
        let targets = parse_targets(
            include_str!("../../../config.example.toml"),
            "config.example.toml",
        )
        .expect("shared config example");

        assert_eq!(targets.len(), 1);
        assert_eq!(targets[0].mac, "aa:bb:cc:dd:ee:ff");
    }

    #[test]
    fn rejects_missing_password() {
        let path = write_config(
            "missing-password",
            r#"
            [[endpoint_sources.static_bmc_endpoints]]
            ip = "10.0.0.1"
            mac = "00:11:22:33:44:55"
            username = "admin"
            "#,
        );

        let error = load_targets(path.to_str().expect("path"))
            .expect_err("missing password should fail")
            .to_string();

        assert!(error.contains("missing field `password`"));
    }

    #[test]
    fn rejects_blank_password() {
        let path = write_config(
            "blank-password",
            r#"
            [[endpoint_sources.static_bmc_endpoints]]
            ip = "10.0.0.1"
            mac = "00:11:22:33:44:55"
            username = "admin"
            password = "   "
            "#,
        );

        let error = load_targets(path.to_str().expect("path"))
            .expect_err("blank password should fail")
            .to_string();

        assert!(error.contains("password is required by the OOB collector"));
    }

    #[test]
    fn rejects_empty_static_endpoint_list() {
        let path = write_config(
            "empty",
            r#"
            [endpoint_sources.carbide_api]
            api_url = "https://api.example.com:1079"
            "#,
        );

        let error = load_targets(path.to_str().expect("path"))
            .expect_err("empty config should fail")
            .to_string();

        assert!(error.contains("static_bmc_endpoints"));
    }

    #[test]
    fn rejects_duplicate_macs() {
        let path = write_config(
            "duplicate-mac",
            r#"
            [[endpoint_sources.static_bmc_endpoints]]
            ip = "10.0.0.1"
            mac = "00:11:22:33:44:55"
            username = "admin"
            password = "secret"
            [[endpoint_sources.static_bmc_endpoints]]
            ip = "10.0.0.2"
            mac = "00:11:22:33:44:55"
            username = "admin"
            password = "secret"
            "#,
        );

        let error = load_targets(path.to_str().expect("path"))
            .expect_err("duplicate MAC should fail")
            .to_string();

        assert!(error.contains("duplicate BMC MAC"));
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
