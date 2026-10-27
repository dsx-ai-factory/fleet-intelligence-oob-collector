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

use std::collections::{BTreeMap, HashSet};
use std::error::Error;
use std::fmt;
use std::net::IpAddr;
use std::path::{Path, PathBuf};

use mac_address::MacAddress;
use serde::Deserialize;

use super::TargetKind;

const MAX_NICO_LABELS: usize = 32;
const MAX_NICO_LABEL_VALUE_BYTES: usize = 1024;
const OOB_LABEL_NAME: &str = "oob";
const OOB_LABEL_VALUE: &str = "true";
const MACHINE_NAME_LABEL_NAME: &str = "machine_name";
const COMPUTE_ZONE_LABEL_NAME: &str = "compute_zone";
const NODE_GROUP_LABEL_NAME: &str = "node_group";
const RESERVED_NICO_LABELS: &[&str] = &[
    "bmc_endpoint",
    "bmc_ip",
    "collector_type",
    "component_type",
    "driver_version",
    "endpoint_ip",
    "endpoint_key",
    "endpoint_mac",
    "gpu_chassis_serial",
    "gpu_model",
    "gpu_serial",
    "gpu_uuid",
    "machine_id",
    "machine_serial",
    "machine_slot_number",
    "machine_tray_index",
    MACHINE_NAME_LABEL_NAME,
    COMPUTE_ZONE_LABEL_NAME,
    NODE_GROUP_LABEL_NAME,
    "nvlink_domain_uuid",
    OOB_LABEL_NAME,
    "power_shelf_id",
    "power_shelf_serial_number",
    "rack_id",
    "serial_number",
    "switch_endpoint",
    "switch_endpoint_role",
    "switch_id",
    "switch_ip",
    "switch_is_primary",
    "switch_serial_number",
    "switch_slot_number",
    "switch_tray_index",
    "system_uuid",
];

#[derive(Clone, Debug)]
pub struct CollectorConfig {
    pub targets: Vec<Target>,
    pub credentials_file: PathBuf,
}

#[derive(Clone)]
pub struct Target {
    pub kind: TargetKind,
    pub name: Option<String>,
    pub compute_zone: Option<String>,
    pub node_group: Option<String>,
    pub ip: IpAddr,
    pub port: Option<u16>,
    pub mac: MacAddress,
    pub address: String,
    pub credentials_profile: Option<String>,
    pub username: String,
    pub password: String,
    pub labels: BTreeMap<String, String>,
}

impl fmt::Debug for Target {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("Target")
            .field("kind", &self.kind)
            .field("name", &self.name)
            .field("compute_zone", &self.compute_zone)
            .field("node_group", &self.node_group)
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
    compute_zone: Option<String>,
    #[serde(default)]
    node_group: Option<String>,
    #[serde(default)]
    labels: BTreeMap<String, String>,
    credentials: CredentialsConfig,
    #[serde(default)]
    targets: Vec<RawTargetConfig>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct CredentialsConfig {
    file: PathBuf,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct RawTargetConfig {
    #[serde(default)]
    kind: Option<TargetKind>,
    #[serde(default)]
    name: Option<String>,
    #[serde(default)]
    compute_zone: Option<String>,
    #[serde(default)]
    node_group: Option<String>,
    ip: IpAddr,
    #[serde(default)]
    port: Option<u16>,
    #[serde(default)]
    mac: Option<String>,
    #[serde(default)]
    credentials_profile: Option<String>,
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

/// Reload credentials for an existing target without changing its configuration.
pub fn load_target_credentials(
    file: &Path,
    mac: &str,
    profile: Option<&str>,
) -> Result<(String, String), Box<dyn Error>> {
    let credentials = SharedCredentials::load(file, Path::new("."))?;
    let resolved = credentials.resolve(mac.parse()?, profile)?;
    Ok((resolved.username, resolved.password))
}

pub(super) fn parse_config(contents: &str, path: &str) -> Result<CollectorConfig, Box<dyn Error>> {
    let config: Config = toml::from_str(contents).map_err(|error| {
        // Invalid input may contain secrets; never echo TOML input.
        let offset = error.span().map(|span| span.start).unwrap_or_default();
        format!(
            "invalid TOML config {path} at byte {offset}; \
             expected [[targets]] and [credentials].file"
        )
    })?;

    if config.targets.is_empty() {
        return Err(format!("no BMC targets found in {path}; configure [[targets]]").into());
    }

    validate_nico_labels("labels", &config.labels)?;
    let default_compute_zone = normalize_config_value("compute_zone", config.compute_zone)?;
    let default_node_group = normalize_config_value("node_group", config.node_group)?;
    let config_dir = Path::new(path).parent().unwrap_or_else(|| Path::new("."));
    // NICo resolves relative credential paths from its working directory. Export
    // the absolute path without resolving symlinks, so secret rotation still works.
    let credentials_file = std::path::absolute(config_dir.join(&config.credentials.file))?;
    let credentials = SharedCredentials::load(&config.credentials.file, config_dir)?;

    let mut macs = HashSet::new();
    let mut targets = Vec::with_capacity(config.targets.len());
    for (index, configured_target) in config.targets.into_iter().enumerate() {
        let kind = configured_target.kind.unwrap_or_default();
        let context = format!("targets[{index}]");
        let mut target = configured_target.into_target(
            kind,
            &context,
            &credentials,
            default_compute_zone.as_deref(),
            default_node_group.as_deref(),
        )?;
        let mut labels = config.labels.clone();
        labels.extend(target.labels);
        validate_nico_labels(&format!("{context}.labels"), &labels)?;
        let internal_label_count = 1
            + usize::from(target.name.is_some())
            + usize::from(target.compute_zone.is_some())
            + usize::from(target.node_group.is_some());
        if labels.len() + internal_label_count > MAX_NICO_LABELS {
            return Err(format!(
                "{context}.labels supports at most {} user-defined NICo labels because {internal_label_count} labels are added internally",
                MAX_NICO_LABELS - internal_label_count
            )
            .into());
        }
        labels.insert(OOB_LABEL_NAME.to_string(), OOB_LABEL_VALUE.to_string());
        if let Some(name) = &target.name {
            labels.insert(MACHINE_NAME_LABEL_NAME.to_string(), name.clone());
        }
        if let Some(compute_zone) = &target.compute_zone {
            labels.insert(COMPUTE_ZONE_LABEL_NAME.to_string(), compute_zone.clone());
        }
        if let Some(node_group) = &target.node_group {
            labels.insert(NODE_GROUP_LABEL_NAME.to_string(), node_group.clone());
        }
        target.labels = labels;
        if !macs.insert(target.mac) {
            return Err(format!("{context}.mac duplicates BMC MAC {}", target.mac).into());
        }
        targets.push(target);
    }

    Ok(CollectorConfig {
        targets,
        credentials_file,
    })
}

fn validate_nico_labels(
    context: &str,
    labels: &BTreeMap<String, String>,
) -> Result<(), Box<dyn Error>> {
    for (name, value) in labels {
        let mut chars = name.chars();
        let valid_start = chars
            .next()
            .is_some_and(|character| character.is_ascii_alphabetic() || character == '_');
        let valid_rest =
            chars.all(|character| character.is_ascii_alphanumeric() || character == '_');
        if !valid_start || !valid_rest {
            return Err(format!("{context} key {name:?} must match [a-zA-Z_][a-zA-Z0-9_]*").into());
        }
        if RESERVED_NICO_LABELS.contains(&name.as_str()) {
            return Err(format!("{context} key {name:?} is reserved by NICo").into());
        }
        if value.len() > MAX_NICO_LABEL_VALUE_BYTES {
            return Err(format!(
                "{context} value for {name:?} exceeds {MAX_NICO_LABEL_VALUE_BYTES} bytes"
            )
            .into());
        }
    }
    Ok(())
}

fn normalize_config_value(
    context: &str,
    value: Option<String>,
) -> Result<Option<String>, Box<dyn Error>> {
    let value = match value {
        Some(value) if value.trim().is_empty() => {
            return Err(format!("{context} cannot be blank").into());
        }
        Some(value) => Some(value.trim().to_string()),
        None => None,
    };
    if value
        .as_ref()
        .is_some_and(|value| value.len() > MAX_NICO_LABEL_VALUE_BYTES)
    {
        return Err(format!("{context} exceeds {MAX_NICO_LABEL_VALUE_BYTES} bytes").into());
    }
    Ok(value)
}

impl RawTargetConfig {
    fn into_target(
        self,
        kind: TargetKind,
        context: &str,
        credentials: &SharedCredentials,
        default_compute_zone: Option<&str>,
        default_node_group: Option<&str>,
    ) -> Result<Target, Box<dyn Error>> {
        let mac = self
            .mac
            .as_deref()
            .ok_or_else(|| format!("{context}.mac is required"))?;
        if mac.trim().is_empty() {
            return Err(format!("{context}.mac cannot be blank").into());
        }
        let mac = mac
            .trim()
            .parse::<MacAddress>()
            .map_err(|error| format!("{context}.mac is invalid: {error}"))?;
        let credentials = credentials.resolve(mac, self.credentials_profile.as_deref())?;
        let name = normalize_config_value(&format!("{context}.name"), self.name)?;
        let compute_zone = normalize_config_value(
            &format!("{context}.compute_zone"),
            self.compute_zone
                .or_else(|| default_compute_zone.map(str::to_string)),
        )?;
        let node_group = normalize_config_value(
            &format!("{context}.node_group"),
            self.node_group
                .or_else(|| default_node_group.map(str::to_string)),
        )?;
        let address = match (self.ip, self.port) {
            (IpAddr::V4(ip), Some(port)) => format!("{ip}:{port}"),
            (IpAddr::V6(ip), Some(port)) => format!("[{ip}]:{port}"),
            (IpAddr::V4(ip), None) => ip.to_string(),
            (IpAddr::V6(ip), None) => format!("[{ip}]"),
        };

        Ok(Target {
            kind,
            name,
            compute_zone,
            node_group,
            ip: self.ip,
            port: self.port,
            mac,
            address,
            credentials_profile: self.credentials_profile,
            username: credentials.username,
            password: credentials.password,
            labels: self.labels,
        })
    }
}

// Same JSON schema and field precedence as NICo health's static endpoint credentials.
// No Debug implementation: credential values must never appear in diagnostics.
#[derive(Default, Deserialize)]
#[serde(default, deny_unknown_fields)]
struct CredentialFields {
    username: Option<String>,
    password: Option<String>,
}

#[derive(Default, Deserialize)]
#[serde(default, deny_unknown_fields)]
struct SharedCredentials {
    profiles: BTreeMap<String, CredentialFields>,
    overrides: BTreeMap<String, CredentialFields>,
}

impl SharedCredentials {
    fn load(path: &Path, config_dir: &Path) -> Result<Self, Box<dyn Error>> {
        let path = if path.is_absolute() {
            path.to_path_buf()
        } else {
            config_dir.join(path)
        };
        let content = std::fs::read_to_string(&path).map_err(|error| {
            format!(
                "failed to read credentials.file {}: {error}",
                path.display()
            )
        })?;
        let mut credentials: Self = serde_json::from_str(&content).map_err(|error| {
            // serde errors can contain secrets (for example, an invalid field type).
            format!(
                "invalid credentials JSON {} at line {}, column {}",
                path.display(),
                error.line(),
                error.column()
            )
        })?;
        let mut normalized = BTreeMap::new();
        for (key, fields) in credentials.overrides {
            let mac = key
                .parse::<MacAddress>()
                .map_err(|_| "invalid MAC key in credentials.file overrides")?;
            if normalized.insert(mac.to_string(), fields).is_some() {
                return Err("duplicate MAC key in credentials.file overrides".into());
            }
        }
        credentials.overrides = normalized;
        Ok(credentials)
    }

    fn resolve(
        &self,
        mac: MacAddress,
        profile: Option<&str>,
    ) -> Result<ResolvedCredentials, Box<dyn Error>> {
        let profile = profile
            .map(|name| {
                self.profiles
                    .get(name)
                    .ok_or_else(|| format!("unknown credentials_profile {name:?}"))
            })
            .transpose()?;
        let entry = self.overrides.get(&mac.to_string());
        let username = entry
            .and_then(|fields| fields.username.clone())
            .or_else(|| profile.and_then(|fields| fields.username.clone()))
            .ok_or_else(|| format!("no username configured for BMC {mac}"))?;
        let password = entry
            .and_then(|fields| fields.password.clone())
            .or_else(|| profile.and_then(|fields| fields.password.clone()))
            .unwrap_or_default();
        Ok(ResolvedCredentials { username, password })
    }
}

/// BMC credentials resolved from the shared JSON secret file.
struct ResolvedCredentials {
    username: String,
    password: String,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_target_kinds() {
        let config = parse_test_config(
            r#"
            [[targets]]
            ip = "10.0.0.1"
            mac = "02:00:0a:00:00:01"
            credentials_profile = "bmc"

            [[targets]]
            kind = "compute"
            ip = "10.0.0.2"
            mac = "02:00:0a:00:00:02"
            credentials_profile = "bmc"

            [[targets]]
            kind = "nvswitch"
            ip = "10.0.0.3"
            mac = "02:00:0a:00:00:03"
            credentials_profile = "bmc"

            [[targets]]
            kind = "power_shelf"
            ip = "10.0.0.4"
            mac = "02:00:0a:00:00:04"
            credentials_profile = "bmc"
            "#,
            "test.toml",
        )
        .expect("config");

        assert_eq!(
            config
                .targets
                .iter()
                .map(|target| target.kind)
                .collect::<Vec<_>>(),
            vec![
                TargetKind::Unknown,
                TargetKind::Compute,
                TargetKind::NvSwitch,
                TargetKind::PowerShelf,
            ]
        );
    }

    #[test]
    fn omitted_target_kind_defaults_to_unknown() {
        let config = parse_test_config(
            r#"
            [[targets]]
            ip = "10.0.0.1"
            mac = "02:00:0a:00:00:01"
            credentials_profile = "bmc"
            "#,
            "test.toml",
        )
        .expect("target kind should be optional");
        assert_eq!(config.targets[0].kind, TargetKind::Unknown);
    }

    #[test]
    fn applies_shared_placement_and_target_label_overrides() {
        let path = write_test_config(
            "valid",
            r#"
            compute_zone = "us-west-dc1"
            node_group = "gb200-training"

            [labels]
            site = "shared"

            [[targets]]
            name = "node-01"
            compute_zone = "us-west-dc2"
            ip = "10.0.0.1"
            port = 8443
            mac = "00:11:22:33:44:55"
            credentials_profile = "bmc"
            labels = { site = "dev", environment = "development" }

            [[targets]]
            ip = "2001:db8::1"
            mac = "00:11:22:33:44:66"
            credentials_profile = "bmc"
            "#,
        );

        let config = load_config(path.to_str().expect("path")).expect("config");

        assert_eq!(config.targets.len(), 2);
        assert_eq!(config.targets[0].name.as_deref(), Some("node-01"));
        assert_eq!(
            config.targets[0].compute_zone.as_deref(),
            Some("us-west-dc2")
        );
        assert_eq!(
            config.targets[0].node_group.as_deref(),
            Some("gb200-training")
        );
        assert_eq!(config.targets[0].mac.to_string(), "00:11:22:33:44:55");
        assert_eq!(config.targets[0].address, "10.0.0.1:8443");
        assert_eq!(
            config.targets[0].labels.get("oob"),
            Some(&"true".to_string())
        );
        assert_eq!(
            config.targets[0].labels.get("site"),
            Some(&"dev".to_string())
        );
        assert_eq!(
            config.targets[0].labels.get("environment"),
            Some(&"development".to_string())
        );
        assert_eq!(
            config.targets[0].labels.get("machine_name"),
            Some(&"node-01".to_string())
        );
        assert_eq!(
            config.targets[0].labels.get("compute_zone"),
            Some(&"us-west-dc2".to_string())
        );
        assert_eq!(config.targets[1].mac.to_string(), "00:11:22:33:44:66");
        assert_eq!(config.targets[1].address, "[2001:db8::1]");
        assert_eq!(
            config.targets[1].compute_zone.as_deref(),
            Some("us-west-dc1")
        );
        assert_eq!(
            config.targets[1].node_group.as_deref(),
            Some("gb200-training")
        );
        assert_eq!(
            config.targets[1].labels.get("oob"),
            Some(&"true".to_string())
        );
        assert_eq!(
            config.targets[1].labels.get("site"),
            Some(&"shared".to_string())
        );
        assert_eq!(
            config.targets[1].labels.get("node_group"),
            Some(&"gb200-training".to_string())
        );
    }

    #[test]
    fn requires_mac_for_targets() {
        for ip in ["10.0.0.10", "2001:db8::102:304"] {
            let input = format!(
                r#"
                [[targets]]
                ip = "{ip}"
                credentials_profile = "bmc"
                "#
            );
            let error = parse_test_config(&input, "test.toml").expect_err("MAC is required");
            assert!(error.to_string().contains("mac is required"));
        }
    }

    #[test]
    fn parses_mac_formats_to_the_same_bytes_and_uppercase_text() {
        for mac in [
            "AA:BB:CC:DD:EE:FF",
            " aa-bb-cc-dd-ee-ff ",
            "aabbccddeeff",
            "AaBbCcDdEeFf",
        ] {
            let input = format!(
                r#"
                [[targets]]
                ip = "10.0.0.10"
                mac = "{mac}"
                credentials_profile = "bmc"
                "#
            );
            let config = parse_test_config(&input, "test.toml").expect("valid MAC");
            assert_eq!(
                config.targets[0].mac.bytes(),
                [0xaa, 0xbb, 0xcc, 0xdd, 0xee, 0xff]
            );
            assert_eq!(config.targets[0].mac.to_string(), "AA:BB:CC:DD:EE:FF");
        }
    }

    #[test]
    fn rejects_invalid_macs_with_entry_context() {
        for mac in [
            "",
            "aa:bb:cc:dd:ee",
            "aa:bb:cc:dd:ee:ff:00",
            "gg:bb:cc:dd:ee:ff",
            "not-a-mac",
        ] {
            let input = format!(
                r#"
                [[targets]]
                ip = "10.0.0.10"
                mac = "{mac}"
                credentials_profile = "bmc"
                "#
            );
            let error = parse_test_config(&input, "test.toml").expect_err("invalid MAC");
            assert!(error.to_string().contains("targets[0].mac"), "{error}");
        }
    }

    #[test]
    fn mac_validation_checks_syntax_only() {
        for mac in [
            "00:00:00:00:00:00",
            "FF:FF:FF:FF:FF:FF",
            "01:00:5E:00:00:01",
        ] {
            let input = format!(
                r#"
                [[targets]]
                ip = "10.0.0.10"
                mac = "{mac}"
                credentials_profile = "bmc"
                "#
            );
            assert!(parse_test_config(&input, "test.toml").is_ok(), "{mac}");
        }
    }

    #[test]
    fn explicit_mac_is_formatted_and_blank_mac_is_rejected() {
        let explicit = parse_test_config(
            r#"
            [[targets]]
            ip = "10.0.0.10"
            mac = " AA-BB-CC-DD-EE-FF "
            credentials_profile = "bmc"
            "#,
            "test.toml",
        )
        .expect("config");
        assert_eq!(explicit.targets[0].mac.to_string(), "AA:BB:CC:DD:EE:FF");

        let error = parse_test_config(
            r#"
            [[targets]]
            ip = "10.0.0.10"
            mac = " "
            credentials_profile = "bmc"
            "#,
            "test.toml",
        )
        .expect_err("blank MAC should fail")
        .to_string();
        assert!(error.contains("mac cannot be blank"));
    }

    #[test]
    fn trims_named_fields_and_rejects_blank_values() {
        let config = parse_test_config(
            r#"
            compute_zone = "  us-west-dc1  "
            node_group = "  shared-group  "

            [[targets]]
            name = "  node-01  "
            node_group = "  gb200-training  "
            ip = "10.0.0.10"
            mac = "02:00:0a:00:00:0a"
            credentials_profile = "bmc"
            "#,
            "test.toml",
        )
        .expect("config");
        assert_eq!(config.targets[0].name.as_deref(), Some("node-01"));
        assert_eq!(
            config.targets[0].compute_zone.as_deref(),
            Some("us-west-dc1")
        );
        assert_eq!(
            config.targets[0].node_group.as_deref(),
            Some("gb200-training")
        );
        assert_eq!(
            config.targets[0]
                .labels
                .get("machine_name")
                .map(String::as_str),
            Some("node-01")
        );

        let error = parse_test_config(
            r#"
            [[targets]]
            name = "   "
            ip = "10.0.0.10"
            mac = "02:00:0a:00:00:0a"
            credentials_profile = "bmc"
            "#,
            "test.toml",
        )
        .expect_err("blank name should fail")
        .to_string();
        assert!(error.contains("name cannot be blank"));

        for field in ["compute_zone", "node_group"] {
            let error = parse_test_config(
                &format!(
                    r#"
                    {field} = "   "

                    [[targets]]
                    ip = "10.0.0.10"
                    mac = "02:00:0a:00:00:0a"
                    credentials_profile = "bmc"
                    "#
                ),
                "test.toml",
            )
            .expect_err("blank placement should fail")
            .to_string();
            assert!(error.contains(&format!("{field} cannot be blank")));
        }
    }

    #[test]
    fn repository_oob_config_example_is_valid() {
        let credentials = write_credential(
            "example-json",
            include_str!("../../../../credentials.example.json"),
        );
        let contents = include_str!("../../../../config.example.toml").replace(
            "/run/secrets/bmc/credentials.json",
            credentials.to_str().unwrap(),
        );
        let config = parse_config(&contents, "config.example.toml").expect("OOB config example");

        assert_eq!(config.targets.len(), 3);
        assert_eq!(config.targets[0].kind, TargetKind::Compute);
        assert_eq!(config.targets[0].name.as_deref(), Some("node-01"));
        assert_eq!(
            config.targets[0].compute_zone.as_deref(),
            Some("us-west-dc1")
        );
        assert_eq!(
            config.targets[0].node_group.as_deref(),
            Some("gb200-training")
        );
        assert_eq!(config.targets[0].mac.to_string(), "AA:BB:CC:DD:EE:FF");
        assert_eq!(
            config.targets[0].labels.get("compute_zone"),
            Some(&"us-west-dc1".to_string())
        );
        assert_eq!(
            config.targets[0].labels.get("node_group"),
            Some(&"gb200-training".to_string())
        );
        assert_eq!(config.targets[1].kind, TargetKind::NvSwitch);
        assert_eq!(config.targets[1].name.as_deref(), Some("switch-tray-01"));
        assert_eq!(config.targets[1].mac.to_string(), "02:00:0A:00:00:0B");
        assert_eq!(config.targets[1].username, "switch-admin");
        assert_eq!(config.targets[2].kind, TargetKind::PowerShelf);
        assert_eq!(config.targets[2].name.as_deref(), Some("power-shelf-01"));
        assert_eq!(config.targets[2].mac.to_string(), "02:00:0A:00:00:0C");
        assert_eq!(config.targets[1].password, "replace-with-target-password");
        std::fs::remove_file(credentials).unwrap();
    }

    #[test]
    fn rejects_empty_target_list() {
        let path = write_test_config("empty", "targets = []");

        let error = load_config(path.to_str().expect("path"))
            .expect_err("empty config should fail")
            .to_string();

        assert!(error.contains("[[targets]]"));
    }

    #[test]
    fn rejects_duplicate_macs_across_formats() {
        for second_mac in ["aa:bb:cc:dd:ee:ff", "AA-BB-CC-DD-EE-FF", "aabbccddeeff"] {
            let input = format!(
                r#"
                [[targets]]
                ip = "10.0.0.1"
                mac = "AA:BB:CC:DD:EE:FF"
                credentials_profile = "bmc"

                [[targets]]
                ip = "10.0.0.2"
                mac = "{second_mac}"
                credentials_profile = "bmc"
                "#
            );
            let error = parse_test_config(&input, "test.toml").expect_err("duplicate MAC");
            assert!(error.to_string().contains("duplicates BMC MAC"), "{error}");
            assert!(error.to_string().contains("targets[1]"), "{error}");
        }
    }

    #[test]
    fn loading_config_keeps_original_mac_text() {
        let path = write_test_config(
            "mac-original-text",
            r#"
            [[targets]]
            ip = "10.0.0.1"
            mac = "aabbccddeeff"
            credentials_profile = "bmc"
            "#,
        );
        let original = std::fs::read_to_string(&path).expect("original config");
        let config = load_config(path.to_str().unwrap()).expect("config");
        assert_eq!(config.targets[0].mac.to_string(), "AA:BB:CC:DD:EE:FF");
        assert_eq!(std::fs::read_to_string(&path).unwrap(), original);
        std::fs::remove_file(path).expect("remove config");
    }

    #[test]
    fn rejects_unknown_fields() {
        let path = write_test_config(
            "unknown",
            r#"
            unexpected = true

            [[targets]]
            ip = "10.0.0.1"
            mac = "00:11:22:33:44:55"
            credentials_profile = "bmc"
            "#,
        );

        let error = load_config(path.to_str().expect("path"))
            .expect_err("unknown field should fail")
            .to_string();

        assert!(error.contains("invalid TOML config"));
    }

    #[test]
    fn rejects_labels_that_override_internal_labels() {
        for label in [
            "compute_zone",
            "gpu_chassis_serial",
            "gpu_model",
            "gpu_serial",
            "gpu_uuid",
            "machine_name",
            "node_group",
            "oob",
            "system_uuid",
        ] {
            let error = parse_test_config(
                &format!(
                    r#"
                    labels = {{ {label} = "override" }}

                    [[targets]]
                    ip = "10.0.0.1"
                    mac = "00:11:22:33:44:55"
                    credentials_profile = "bmc"
                    "#
                ),
                "test.toml",
            )
            .expect_err("reserved label should fail")
            .to_string();

            assert!(error.contains(label));
        }
    }
    #[test]
    fn shared_json_credentials_follow_nico_profile_and_mac_precedence() {
        let path = write_credential(
            "shared-json-precedence",
            r#"{
            "profiles": {
                "bmc": {"username":"admin","password":"shared"},
                "switch": {"username":"switch-admin","password":"switch-pass"}
            },
            "overrides": {
                "aa:bb:cc:dd:ee:01": {"password":"individual"},
                "aa:bb:cc:dd:ee:02": {"username":"standalone"},
                "aa:bb:cc:dd:ee:03": {"password":""},
                "aa:bb:cc:dd:ee:04": {"username":null,"password":null},
                "aa:bb:cc:dd:ee:05": {"username":""},
                "aa:bb:cc:dd:ee:06": {"username":"node-admin","password":"node-pass"}
            }
        }"#,
        );
        for (mac, profile, username, password) in [
            (
                "AA:BB:CC:DD:EE:00",
                Some("switch"),
                "switch-admin",
                "switch-pass",
            ),
            ("AA:BB:CC:DD:EE:01", Some("bmc"), "admin", "individual"),
            ("AA:BB:CC:DD:EE:02", None, "standalone", ""),
            ("AA:BB:CC:DD:EE:02", Some("bmc"), "standalone", "shared"),
            ("AA:BB:CC:DD:EE:03", Some("bmc"), "admin", ""),
            ("AA:BB:CC:DD:EE:04", Some("bmc"), "admin", "shared"),
            ("AA:BB:CC:DD:EE:05", Some("bmc"), "", "shared"),
            ("AA:BB:CC:DD:EE:06", Some("bmc"), "node-admin", "node-pass"),
            ("AA:BB:CC:DD:EE:06", None, "node-admin", "node-pass"),
        ] {
            let profile = profile
                .map(|name| format!("credentials_profile = {name:?}"))
                .unwrap_or_default();
            let config = parse_config(
                &format!(
                    r#"
                [credentials]
                file = {:?}
                [[targets]]
                ip = "192.0.2.1"
                mac = {mac:?}
                {profile}
            "#,
                    path.to_str().unwrap()
                ),
                "test.toml",
            )
            .expect("shared JSON config");
            assert_eq!(config.targets[0].username, username);
            assert_eq!(config.targets[0].password, password);
            assert!(!format!("{:?}", config.targets[0]).contains("individual"));
        }
        for (profile, expected) in [
            ("", "no username configured"),
            (
                "credentials_profile = 'missing'",
                "unknown credentials_profile",
            ),
        ] {
            let error = parse_config(
                &format!(
                    r#"
                [credentials]
                file = {:?}
                [[targets]]
                ip = "192.0.2.1"
                mac = "AA:BB:CC:DD:EE:00"
                {profile}
            "#,
                    path.to_str().unwrap()
                ),
                "test.toml",
            )
            .unwrap_err()
            .to_string();
            assert!(error.contains(expected), "{error}");
        }
        std::fs::remove_file(path).unwrap();
    }

    #[test]
    fn shared_json_credentials_reject_invalid_snapshots_without_leaking_values() {
        let path = write_credential("shared-json-invalid", "{}");
        for input in [
            r#"{"profiles":{"bmc":{"password":["sensitive-value"]}}}"#,
            r#"{"profiles":{"bmc":{"pasword":"sensitive-value"}}}"#,
            r#"{"overrides":{"sensitive-value":{"password":"sensitive-value"}}}"#,
            r#"{"overrides":{"AA:BB:CC:DD:EE:FF":{},"aa:bb:cc:dd:ee:ff":{}}}"#,
            r#"{"profiles":{"bmc":{"password":"sensitive-value"}}"#,
            r#"{"unexpected":"sensitive-value"}"#,
        ] {
            std::fs::write(&path, input).unwrap();
            let error = SharedCredentials::load(&path, Path::new("."))
                .err()
                .expect("invalid snapshot");
            assert!(!error.to_string().contains("sensitive-value"));
        }
        std::fs::remove_file(path).unwrap();
    }

    #[test]
    fn shared_json_credentials_reload_atomic_replacements_and_relative_paths() {
        let path = write_credential(
            "shared-json-rotation",
            r#"{"profiles":{"bmc":{"username":"admin","password":"first"}}}"#,
        );
        let config = write_config(
            "shared-json-rotation",
            &format!(
                r#"
            [credentials]
            file = {:?}
            [[targets]]
            ip = "192.0.2.1"
            mac = "00:11:22:33:44:55"
            credentials_profile = "bmc"
        "#,
                path.file_name().unwrap().to_str().unwrap()
            ),
        );
        assert_eq!(
            load_config(config.to_str().unwrap()).unwrap().targets[0].password,
            "first"
        );
        let replacement = path.with_extension("next");
        std::fs::write(
            &replacement,
            r#"{"profiles":{"bmc":{"username":"admin","password":"second"}}}"#,
        )
        .unwrap();
        std::fs::rename(replacement, &path).unwrap();
        assert_eq!(
            load_config(config.to_str().unwrap()).unwrap().targets[0].password,
            "second"
        );
        std::fs::remove_file(config).unwrap();
        std::fs::remove_file(path).unwrap();
    }

    #[test]
    fn credential_refresh_keeps_profile_and_mac_override_precedence() {
        let file = write_credential(
            "refresh-credentials",
            r#"{"profiles":{"bmc":{"username":"old","password":"old"}}}"#,
        );
        assert_eq!(
            load_target_credentials(&file, "00:11:22:33:44:55", Some("bmc")).unwrap(),
            ("old".into(), "old".into())
        );
        let replacement = file.with_extension("next");
        std::fs::write(
            &replacement,
            r#"{
            "profiles":{"bmc":{"username":"new","password":"shared"}},
            "overrides":{"00:11:22:33:44:55":{"password":"individual"}}
        }"#,
        )
        .unwrap();
        std::fs::rename(replacement, &file).unwrap();
        assert_eq!(
            load_target_credentials(&file, "00:11:22:33:44:55", Some("bmc")).unwrap(),
            ("new".into(), "individual".into())
        );
        std::fs::write(
            &file,
            r#"{"profiles":{"bmc":{"password":["private-value"]}}}"#,
        )
        .unwrap();
        let error = load_target_credentials(&file, "00:11:22:33:44:55", Some("bmc")).unwrap_err();
        assert!(!error.to_string().contains("private-value"));
        std::fs::remove_file(file).unwrap();
    }

    #[test]
    fn requires_secret_file_and_rejects_unsupported_config_fields() {
        for field in ["username", "password", "username_file", "password_file"] {
            let input = format!(
                "[credentials]\nfile = 'unused.json'\n[[targets]]\nip = '192.0.2.1'\nmac = '00:11:22:33:44:55'\n{field} = 'sensitive-value'"
            );
            let error = parse_config(&input, "test.toml").unwrap_err().to_string();
            assert!(error.contains("invalid TOML config"), "{error}");
            assert!(!error.contains("sensitive-value"));
        }
        for input in [
            "[credentials]\nfile = 'unused.json'\nusername_file = 'sensitive-value'\npassword_file = 'sensitive-value'",
            "[credentials]\nfile = 'unused.json'\n[[machines]]\nip = '192.0.2.1'\nmac = '00:11:22:33:44:55'",
            "[[targets]]\nip = '192.0.2.1'\nmac = '00:11:22:33:44:55'\ncredentials_profile = 'bmc'",
        ] {
            let error = parse_config(input, "test.toml").unwrap_err().to_string();
            assert!(error.contains("invalid TOML config"), "{error}");
            assert!(!error.contains("sensitive-value"));
        }
        let input = "[credentials]\nfile = 'missing-secret-file.json'\n[[targets]]\nip = '192.0.2.1'\nmac = '00:11:22:33:44:55'";
        assert!(
            parse_config(input, "test.toml")
                .unwrap_err()
                .to_string()
                .contains("failed to read credentials.file")
        );
    }

    // Metadata tests use the same file-only credential path as production.
    fn with_test_credentials(content: &str) -> String {
        static SECRET: std::sync::OnceLock<PathBuf> = std::sync::OnceLock::new();
        let path = SECRET.get_or_init(|| {
            write_credential(
                "metadata-credentials",
                r#"{"profiles":{"bmc":{"username":"admin","password":"secret"}}}"#,
            )
        });
        format!(
            "{content}\n[credentials]\nfile = {:?}",
            path.to_str().unwrap()
        )
    }

    fn parse_test_config(contents: &str, path: &str) -> Result<CollectorConfig, Box<dyn Error>> {
        parse_config(&with_test_credentials(contents), path)
    }

    fn write_test_config(name: &str, content: &str) -> PathBuf {
        write_config(name, &with_test_credentials(content))
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

    fn write_credential(name: &str, content: &str) -> std::path::PathBuf {
        let mut path = std::env::temp_dir();
        path.push(format!(
            "fleet-intelligence-oob-{name}-{}",
            std::process::id()
        ));
        std::fs::write(&path, content).expect("write credential");
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
