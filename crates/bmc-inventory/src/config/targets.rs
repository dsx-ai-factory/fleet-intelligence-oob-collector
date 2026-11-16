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

use super::{NicoHealthSettings, TargetKind};
use crate::endpoint::nico::{NicoRestConfig, NicoRestEndpointSource};
use crate::endpoint::{Endpoint, EndpointSource, StaticEndpointSource};

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
    pub nico_health: NicoHealthSettings,
}

/// Selected endpoint source paired with shared settings, before discovery.
pub struct SourceConfig {
    pub source: Box<dyn EndpointSource>,
    pub settings: EndpointConfig,
}

/// Runtime-scoped, credential-free discovery fallback. It is never persisted.
#[derive(Default)]
pub struct RuntimeConfigLoader {
    last_good: Option<LastGoodDiscovery>,
}

struct LastGoodDiscovery {
    source: NicoRestConfig,
    endpoints: crate::endpoint::EndpointSnapshot,
    discovered_at: std::time::Instant,
}

/// True only for discovery availability failures, not configuration or identity errors.
pub fn is_discovery_unavailable(error: &(dyn Error + 'static)) -> bool {
    error.is::<crate::endpoint::nico::DiscoveryUnavailable>()
}

impl RuntimeConfigLoader {
    /// Reload settings and credentials, using a bounded last-good discovery only
    /// when the same NICo scope is unavailable. Static configuration never caches.
    pub async fn load(&mut self, path: &str) -> Result<CollectorConfig, Box<dyn Error>> {
        let contents = tokio::fs::read_to_string(path).await?;
        let raw = parse_raw_config(&contents, path)?;
        if matches!(raw.endpoint_source, SourceSelection::Static) {
            self.last_good = None;
            return load_config_async(path).await;
        }
        let parsed = prepare_config(raw, path)?;
        let SourceSelection::Nico(source_config) = &parsed.source else {
            unreachable!()
        };
        let source_config = source_config.clone();
        let selected = parsed.into_source()?;
        let discovered = selected.source.discover().await;
        self.resolve_discovery(&selected.settings, source_config, discovered)
            .await
    }

    async fn resolve_discovery(
        &mut self,
        settings: &EndpointConfig,
        source: NicoRestConfig,
        discovered: Result<crate::endpoint::EndpointSnapshot, crate::endpoint::EndpointSourceError>,
    ) -> Result<CollectorConfig, Box<dyn Error>> {
        let (endpoints, fresh) = match discovered {
            Ok(endpoints) => {
                // An authoritative result supersedes old membership immediately,
                // including empty results and results failing shared validation.
                self.last_good = None;
                (endpoints, true)
            }
            Err(error) if is_discovery_unavailable(error.as_ref()) => {
                let cached = self.last_good.as_ref().filter(|cached| {
                    cached.source == source
                        && cached.discovered_at.elapsed().as_secs() < source.max_stale_seconds
                });
                let Some(cached) = cached else {
                    self.last_good = None;
                    return Err(error as Box<dyn Error>);
                };
                tracing::warn!(age_seconds = cached.discovered_at.elapsed().as_secs(), max_stale_seconds = source.max_stale_seconds, targets = cached.endpoints.len(), reason = %error, "NICo unavailable; using bounded last-good discovery");
                (cached.endpoints.clone(), false)
            }
            Err(error) => {
                self.last_good = None;
                return Err(error as Box<dyn Error>);
            }
        };
        require_endpoints(&endpoints)?;
        // Never cache BMC passwords or fallback around credential/label errors.
        let credentials = SharedCredentials::load_async(&settings.credentials_file).await?;
        let resolved = settings.resolve_endpoints(&endpoints, &credentials)?;
        if fresh {
            self.last_good = Some(LastGoodDiscovery {
                source,
                endpoints,
                discovered_at: std::time::Instant::now(),
            });
        }
        Ok(resolved)
    }
}

/// Static endpoint definitions paired with provider-independent settings.
#[derive(Clone, Debug)]
pub struct StaticConfig {
    pub source: StaticEndpointSource,
    pub settings: EndpointConfig,
}

impl StaticConfig {
    /// Discover and resolve one static target snapshot for both workflows.
    pub async fn resolve(self) -> Result<CollectorConfig, Box<dyn Error>> {
        self.settings.resolve_from(&self.source).await
    }
}

/// Shared collection settings, independent of static targets or provider type.
/// Load this with `load_endpoint_config` for API-only endpoint discovery.
#[derive(Clone, Debug)]
pub struct EndpointConfig {
    // Absolute and rooted at the config directory. Private so a relative path
    // cannot be substituted and accidentally resolved from the working directory.
    credentials_file: PathBuf,
    pub nico_health: NicoHealthSettings,
    defaults: EndpointDefaults,
}

#[derive(Clone, Debug)]
struct EndpointDefaults {
    compute_zone: Option<String>,
    node_group: Option<String>,
    labels: BTreeMap<String, String>,
}

impl EndpointConfig {
    /// Discover from any provider and apply shared Fleet normalization and
    /// credentials. No static target definitions are needed.
    pub async fn resolve_from(
        &self,
        source: &dyn EndpointSource,
    ) -> Result<CollectorConfig, Box<dyn Error>> {
        let endpoints = source
            .discover()
            .await
            .map_err(|error| error as Box<dyn Error>)?;
        require_endpoints(&endpoints)?;
        let credentials = SharedCredentials::load_async(&self.credentials_file).await?;
        self.resolve_endpoints(&endpoints, &credentials)
    }

    fn resolve_endpoints(
        &self,
        endpoints: &[Endpoint],
        credentials: &SharedCredentials,
    ) -> Result<CollectorConfig, Box<dyn Error>> {
        Ok(CollectorConfig {
            targets: resolve_endpoints(endpoints, credentials, &self.defaults)?,
            credentials_file: self.credentials_file.clone(),
            nico_health: self.nico_health.clone(),
        })
    }
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
    nico_health: NicoHealthSettings,
    #[serde(default)]
    compute_zone: Option<String>,
    #[serde(default)]
    node_group: Option<String>,
    #[serde(default)]
    labels: BTreeMap<String, String>,
    credentials: CredentialsConfig,
    #[serde(default)]
    targets: Vec<RawTargetConfig>,
    #[serde(default)]
    endpoint_source: SourceSelection,
}

#[derive(Default, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case", deny_unknown_fields)]
enum SourceSelection {
    #[default]
    Static,
    Nico(NicoRestConfig),
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

/// Load the selected source without resolving BMC credentials or contacting it.
pub async fn load_source_config(path: &str) -> Result<SourceConfig, Box<dyn Error>> {
    let contents = tokio::fs::read_to_string(path).await?;
    parse_config_parts(&contents, path)?.into_source()
}

/// Read shared settings without requiring or constructing static targets.
pub fn load_endpoint_config(path: &str) -> Result<EndpointConfig, Box<dyn Error>> {
    let contents = std::fs::read_to_string(path)?;
    Ok(parse_config_parts(&contents, path)?.settings)
}

/// Asynchronously load shared settings for an API-only deployment.
pub async fn load_endpoint_config_async(path: &str) -> Result<EndpointConfig, Box<dyn Error>> {
    let contents = tokio::fs::read_to_string(path).await?;
    Ok(parse_config_parts(&contents, path)?.settings)
}

/// Read static endpoint definitions without loading BMC credential values.
pub fn load_static_config(path: &str) -> Result<StaticConfig, Box<dyn Error>> {
    let contents = std::fs::read_to_string(path)?;
    parse_static_config(&contents, path)
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

/// Runtime loader: use async file I/O and preserve static configuration's
/// credentials-file-before-target-validation error precedence.
pub async fn load_config_async(path: &str) -> Result<CollectorConfig, Box<dyn Error>> {
    let contents = tokio::fs::read_to_string(path).await?;
    let raw = parse_raw_config(&contents, path)?;
    if !matches!(raw.endpoint_source, SourceSelection::Static) {
        let selected = prepare_config(raw, path)?.into_source()?;
        return selected
            .settings
            .resolve_from(selected.source.as_ref())
            .await;
    }
    require_static_targets(&raw, path)?;
    let parsed = prepare_config(raw, path)?;
    let credentials = SharedCredentials::load_async(&parsed.settings.credentials_file).await?;
    let config = parsed.into_static()?;
    let endpoints = config
        .source
        .discover()
        .await
        .map_err(|error| error as Box<dyn Error>)?;
    config.settings.resolve_endpoints(&endpoints, &credentials)
}

pub(super) fn parse_config(contents: &str, path: &str) -> Result<CollectorConfig, Box<dyn Error>> {
    let parsed = parse_static_parts(contents, path)?;
    let credentials = SharedCredentials::load(&parsed.settings.credentials_file, Path::new("."))?;
    let config = parsed.into_static()?;
    config
        .settings
        .resolve_endpoints(config.source.endpoints(), &credentials)
}

fn parse_static_config(contents: &str, path: &str) -> Result<StaticConfig, Box<dyn Error>> {
    parse_static_parts(contents, path)?.into_static()
}

struct ParsedConfig {
    targets: Vec<RawTargetConfig>,
    settings: EndpointConfig,
    source: SourceSelection,
}

fn parse_static_parts(contents: &str, path: &str) -> Result<ParsedConfig, Box<dyn Error>> {
    let config = parse_raw_config(contents, path)?;
    require_static_targets(&config, path)?;
    prepare_config(config, path)
}

fn require_static_targets(config: &Config, path: &str) -> Result<(), Box<dyn Error>> {
    if !matches!(config.endpoint_source, SourceSelection::Static) {
        return Err("render-nico-config requires a separate static targets config for NICo health; use FLEETINT_COLLECTOR_CONFIG for collector-only API discovery and bmc-collector discover to check it (supports static targets only)".into());
    }
    if config.targets.is_empty() {
        return Err(format!("no BMC targets found in {path}; configure [[targets]]").into());
    }
    Ok(())
}

impl ParsedConfig {
    fn into_source(self) -> Result<SourceConfig, Box<dyn Error>> {
        match self.source {
            SourceSelection::Nico(config) => Ok(SourceConfig {
                source: Box::new(
                    NicoRestEndpointSource::new(config).map_err(|error| error as Box<dyn Error>)?,
                ),
                settings: self.settings,
            }),
            SourceSelection::Static => {
                if self.targets.is_empty() {
                    return Err("no BMC targets found; configure [[targets]] for the static endpoint source".into());
                }
                let config = self.into_static()?;
                Ok(SourceConfig {
                    source: Box::new(config.source),
                    settings: config.settings,
                })
            }
        }
    }

    fn into_static(self) -> Result<StaticConfig, Box<dyn Error>> {
        let endpoints = self
            .targets
            .into_iter()
            .enumerate()
            .map(|(index, target)| target.into_endpoint(&format!("targets[{index}]")))
            .collect::<Result<Vec<_>, _>>()?;
        Ok(StaticConfig {
            source: StaticEndpointSource::new(endpoints),
            settings: self.settings,
        })
    }
}

fn parse_config_parts(contents: &str, path: &str) -> Result<ParsedConfig, Box<dyn Error>> {
    prepare_config(parse_raw_config(contents, path)?, path)
}

fn parse_raw_config(contents: &str, path: &str) -> Result<Config, Box<dyn Error>> {
    toml::from_str(contents).map_err(|error: toml::de::Error| {
        // Invalid input may contain secrets; never echo TOML input.
        let offset = error.span().map(|span| span.start).unwrap_or_default();
        format!("invalid TOML config {path} at byte {offset}; expected [credentials].file and optional [[targets]]").into()
    })
}

fn prepare_config(mut config: Config, path: &str) -> Result<ParsedConfig, Box<dyn Error>> {
    if matches!(config.endpoint_source, SourceSelection::Nico(_)) && !config.targets.is_empty() {
        return Err("NICo REST endpoint_source cannot be combined with [[targets]]".into());
    }
    config.nico_health.validate()?;
    validate_nico_labels("labels", &config.labels)?;
    let defaults = EndpointDefaults {
        compute_zone: normalize_config_value("compute_zone", config.compute_zone)?,
        node_group: normalize_config_value("node_group", config.node_group)?,
        labels: config.labels,
    };
    let config_dir = Path::new(path).parent().unwrap_or_else(|| Path::new("."));
    if let SourceSelection::Nico(source) = &mut config.endpoint_source {
        if source.token_file.as_os_str().is_empty() {
            return Err("NICo token_file must not be empty".into());
        }
        source.token_file = std::path::absolute(config_dir.join(&source.token_file))?;
    }
    // Keep symlinks intact so atomic credential-file replacement still works.
    let credentials_file = std::path::absolute(config_dir.join(&config.credentials.file))?;
    Ok(ParsedConfig {
        targets: config.targets,
        source: config.endpoint_source,
        settings: EndpointConfig {
            credentials_file,
            nico_health: config.nico_health,
            defaults,
        },
    })
}

fn require_endpoints(endpoints: &[Endpoint]) -> Result<(), Box<dyn Error>> {
    if endpoints.is_empty() {
        return Err("no BMC targets found in endpoint source".into());
    }
    Ok(())
}

// The single discovery-to-target boundary for every provider. Endpoints contain
// only raw/user labels; normalized fields and derived labels exist on Target.
fn resolve_endpoints(
    endpoints: &[Endpoint],
    credentials: &SharedCredentials,
    defaults: &EndpointDefaults,
) -> Result<Vec<Target>, Box<dyn Error>> {
    require_endpoints(endpoints)?;
    let mut macs = HashSet::new();
    let mut targets = Vec::with_capacity(endpoints.len());
    for (index, endpoint) in endpoints.iter().enumerate() {
        let context = match &endpoint.source_reference {
            Some(reference) => format!(
                "targets[{index}] (source {})",
                crate::endpoint::safe_source_reference(reference)
            ),
            None => format!("targets[{index}]"),
        };
        let identity = format!("{context} (BMC {})", endpoint.mac);
        let resolved = credentials
            .resolve(endpoint.mac, endpoint.credentials_profile.as_deref())
            .map_err(|error| format!("{identity}: {error}"))?;
        let name = normalize_config_value(&format!("{context}.name"), endpoint.name.clone())?;
        let compute_zone = normalize_config_value(
            &format!("{context}.compute_zone"),
            endpoint
                .compute_zone
                .clone()
                .or_else(|| defaults.compute_zone.clone()),
        )?;
        let node_group = normalize_config_value(
            &format!("{context}.node_group"),
            endpoint
                .node_group
                .clone()
                .or_else(|| defaults.node_group.clone()),
        )?;
        let mut labels = defaults.labels.clone();
        labels.extend(endpoint.labels.clone());
        validate_nico_labels(&format!("{context}.labels"), &labels)?;
        let internal_label_count = 1
            + usize::from(name.is_some())
            + usize::from(compute_zone.is_some())
            + usize::from(node_group.is_some());
        if labels.len() + internal_label_count > MAX_NICO_LABELS {
            return Err(format!(
                "{context}.labels supports at most {} user-defined NICo labels because {internal_label_count} labels are added internally",
                MAX_NICO_LABELS - internal_label_count
            ).into());
        }
        labels.insert(OOB_LABEL_NAME.into(), OOB_LABEL_VALUE.into());
        for (key, value) in [
            (MACHINE_NAME_LABEL_NAME, &name),
            (COMPUTE_ZONE_LABEL_NAME, &compute_zone),
            (NODE_GROUP_LABEL_NAME, &node_group),
        ] {
            if let Some(value) = value {
                labels.insert(key.into(), value.clone());
            }
        }
        if !macs.insert(endpoint.mac) {
            return Err(format!("{identity}: duplicates BMC MAC {}", endpoint.mac).into());
        }
        targets.push(Target {
            kind: endpoint.kind,
            name,
            compute_zone,
            node_group,
            ip: endpoint.ip,
            port: endpoint.port,
            mac: endpoint.mac,
            address: endpoint.address(),
            credentials_profile: endpoint.credentials_profile.clone(),
            username: resolved.username,
            password: resolved.password,
            labels,
        });
    }
    Ok(targets)
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
    fn into_endpoint(self, context: &str) -> Result<Endpoint, Box<dyn Error>> {
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
        Ok(Endpoint {
            source_reference: None,
            kind: self.kind.unwrap_or_default(),
            name: self.name,
            compute_zone: self.compute_zone,
            node_group: self.node_group,
            ip: self.ip,
            port: self.port,
            mac,
            credentials_profile: self.credentials_profile,
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
        Self::parse(&content, &path)
    }

    async fn load_async(path: &Path) -> Result<Self, Box<dyn Error>> {
        let content = tokio::fs::read_to_string(path).await.map_err(|error| {
            format!(
                "failed to read credentials.file {}: {error}",
                path.display()
            )
        })?;
        Self::parse(&content, path)
    }

    fn parse(content: &str, path: &Path) -> Result<Self, Box<dyn Error>> {
        let mut credentials: Self = serde_json::from_str(content).map_err(|error| {
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

    #[tokio::test]
    async fn static_source_preserves_resolved_configuration() {
        let contents = with_test_credentials(
            r#"
            compute_zone = "zone"
            node_group = "group"
            labels = { site = "example" }
            [[targets]]
            kind = "compute"
            name = "host"
            ip = "2001:db8::1"
            port = 8443
            mac = "aa:bb:cc:dd:ee:01"
            credentials_profile = "bmc"
            [[targets]]
            kind = "nvswitch"
            ip = "192.0.2.2"
            mac = "aa:bb:cc:dd:ee:02"
            credentials_profile = "bmc"
            node_group = "switches"
            "#,
        );
        let expected = parse_config(&contents, "config.toml").unwrap();
        let config = parse_static_config(&contents, "config.toml").unwrap();
        let actual = config.resolve().await.unwrap();
        assert_eq!(actual.credentials_file, expected.credentials_file);
        assert_eq!(actual.targets.len(), expected.targets.len());
        for (actual, expected) in actual.targets.iter().zip(&expected.targets) {
            assert_eq!(actual.kind, expected.kind);
            assert_eq!(actual.name, expected.name);
            assert_eq!(actual.compute_zone, expected.compute_zone);
            assert_eq!(actual.node_group, expected.node_group);
            assert_eq!(actual.ip, expected.ip);
            assert_eq!(actual.port, expected.port);
            assert_eq!(actual.mac, expected.mac);
            assert_eq!(actual.address, expected.address);
            assert_eq!(actual.labels, expected.labels);
            assert_eq!(actual.credentials_profile, expected.credentials_profile);
            assert_eq!(actual.username, expected.username);
            assert_eq!(actual.password, expected.password);
        }
    }

    #[tokio::test]
    async fn discovery_precedes_credentials_and_source_errors_abort_resolution() {
        struct FailedSource;
        #[async_trait::async_trait]
        impl EndpointSource for FailedSource {
            async fn discover(
                &self,
            ) -> Result<crate::endpoint::EndpointSnapshot, crate::endpoint::EndpointSourceError>
            {
                Err("discovery unavailable".into())
            }
        }

        let contents = "[credentials]\nfile = 'unavailable-credentials.json'\n[[targets]]\nip = '192.0.2.1'\nmac = '00:11:22:33:44:55'";
        let config = parse_static_config(contents, "config.toml").unwrap();
        let source: &dyn EndpointSource = &config.source;
        let mut first = source.discover().await.unwrap();
        let second = source.discover().await.unwrap();
        assert!(std::sync::Arc::ptr_eq(&first, &second));
        std::sync::Arc::make_mut(&mut first)[0].name = Some("changed".into());
        assert!(source.discover().await.unwrap()[0].name.is_none());
        let error = config
            .settings
            .resolve_from(&FailedSource)
            .await
            .unwrap_err();
        assert_eq!(error.to_string(), "discovery unavailable");
        assert!(
            config
                .resolve()
                .await
                .unwrap_err()
                .to_string()
                .contains("failed to read credentials.file")
        );
    }

    #[tokio::test]
    async fn provider_listing_uses_shared_credential_overrides_and_duplicate_validation() {
        let file = write_credential(
            "endpoint-provider",
            r#"{"profiles":{"bmc":{"username":"shared","password":"shared"}},"overrides":{"00:11:22:33:44:66":{"password":"individual"}}}"#,
        );
        let _cleanup = TestFileCleanup(file.clone());
        let input = format!(
            "[credentials]\nfile = {:?}\n[[targets]]\nip = '192.0.2.1'\nmac = '00:11:22:33:44:55'\ncredentials_profile = 'bmc'",
            file.to_str().unwrap()
        );
        let config = parse_static_config(&input, "config.toml").unwrap();
        let mut endpoint = config.source.endpoints()[0].clone();
        endpoint.mac = "00:11:22:33:44:66".parse().unwrap();
        endpoint.ip = "192.0.2.2".parse().unwrap();
        let provider = StaticEndpointSource::new(vec![endpoint.clone()]);
        let resolved = config.settings.resolve_from(&provider).await.unwrap();
        assert_eq!(resolved.targets.len(), 1);
        assert_eq!(resolved.targets[0].ip, endpoint.ip);
        assert_eq!(resolved.targets[0].username, "shared");
        assert_eq!(resolved.targets[0].password, "individual");
        let duplicate = StaticEndpointSource::new(vec![endpoint.clone(), endpoint]);
        assert!(
            config
                .settings
                .resolve_from(&duplicate)
                .await
                .unwrap_err()
                .to_string()
                .contains("duplicates BMC MAC")
        );
    }

    // This intentionally implements the interface independently of the static
    // adapter so provider tests exercise the shared boundary, not TOML mapping.
    struct ProviderSource(crate::endpoint::EndpointSnapshot);

    #[async_trait::async_trait]
    impl EndpointSource for ProviderSource {
        async fn discover(
            &self,
        ) -> Result<crate::endpoint::EndpointSnapshot, crate::endpoint::EndpointSourceError>
        {
            Ok(self.0.clone())
        }
    }

    fn provider_endpoint() -> Endpoint {
        Endpoint {
            source_reference: None,
            kind: TargetKind::Compute,
            name: Some("original".into()),
            compute_zone: Some("original-zone".into()),
            node_group: Some("original-group".into()),
            ip: "192.0.2.1".parse().unwrap(),
            port: None,
            mac: "00:11:22:33:44:55".parse().unwrap(),
            credentials_profile: Some("bmc".into()),
            labels: BTreeMap::from([("rack".into(), "rack-1".into())]),
        }
    }

    #[tokio::test]
    async fn provider_only_config_applies_shared_settings_and_finalization() {
        let contents = with_test_credentials(
            r#"
            compute_zone = "default-zone"
            node_group = "default-group"
            labels = { site = "shared", facility = "dc1" }
            [nico_health]
            sensor_fetch_interval_seconds = 120
            "#,
        );
        let path = write_config("provider-only", &contents);
        let _cleanup = TestFileCleanup(path.clone());
        let path = path.to_str().unwrap();
        // Both public shared-settings loaders work with no [[targets]] at all.
        let config = load_endpoint_config_async(path).await.unwrap();
        let sync_config = load_endpoint_config(path).unwrap();
        assert_eq!(config.credentials_file, sync_config.credentials_file);
        assert!(
            load_static_config(path)
                .unwrap_err()
                .to_string()
                .contains("no BMC targets")
        );
        let original = provider_endpoint();
        assert!(!original.labels.contains_key("oob"));
        assert!(!original.labels.contains_key("machine_name"));
        let mut endpoint = original.clone();
        endpoint.name = Some(" new-name ".into());
        endpoint.compute_zone = Some(" new-zone ".into());
        endpoint.node_group = None;
        endpoint.labels.insert("site".into(), "provider".into());
        let source = ProviderSource(vec![endpoint.clone()].into());
        let resolved = config.resolve_from(&source).await.unwrap();
        assert_eq!(
            resolved
                .nico_health
                .sensor_fetch_interval_seconds
                .unwrap()
                .get(),
            120
        );
        let target = &resolved.targets[0];
        assert_eq!(target.name.as_deref(), Some("new-name"));
        assert_eq!(target.compute_zone.as_deref(), Some("new-zone"));
        assert_eq!(target.node_group.as_deref(), Some("default-group"));
        for (key, value) in [
            ("oob", "true"),
            ("machine_name", "new-name"),
            ("compute_zone", "new-zone"),
            ("node_group", "default-group"),
            ("site", "provider"),
            ("facility", "dc1"),
            ("rack", "rack-1"),
        ] {
            assert_eq!(target.labels.get(key).map(String::as_str), Some(value));
        }
        // Clearing the name on the next snapshot cannot retain a derived label.
        endpoint.name = None;
        let source = ProviderSource(vec![endpoint].into());
        let resolved = config.resolve_from(&source).await.unwrap();
        assert!(!resolved.targets[0].labels.contains_key("machine_name"));
        assert_eq!(original.name.as_deref(), Some("original"));
    }

    #[tokio::test]
    async fn provider_label_validation_and_credential_errors_have_target_context() {
        let contents = with_test_credentials("");
        let config = parse_config_parts(&contents, "config.toml")
            .unwrap()
            .settings;
        let endpoint = provider_endpoint();
        let cases = [
            (BTreeMap::from([("oob".into(), "false".into())]), "reserved"),
            (
                BTreeMap::from([("invalid-key".into(), "value".into())]),
                "must match",
            ),
            (
                BTreeMap::from([("key".into(), "x".repeat(1025))]),
                "exceeds",
            ),
            (
                (0..32)
                    .map(|i| (format!("key_{i}"), "value".into()))
                    .collect(),
                "at most",
            ),
        ];
        for (labels, expected) in cases {
            let mut endpoint = endpoint.clone();
            endpoint.labels = labels;
            let source = ProviderSource(vec![endpoint].into());
            let error = config.resolve_from(&source).await.unwrap_err().to_string();
            assert!(error.contains(expected), "{error}");
            assert!(error.contains("targets[0].labels"), "{error}");
        }
        let mut endpoint = endpoint.clone();
        endpoint.credentials_profile = Some("missing".into());
        let source = ProviderSource(vec![endpoint].into());
        let error = config.resolve_from(&source).await.unwrap_err().to_string();
        assert!(error.contains("targets[0]"));
        assert!(error.contains("00:11:22:33:44:55"));
        assert!(error.contains("unknown credentials_profile"));
    }

    #[tokio::test]
    async fn empty_provider_is_rejected_before_reading_credentials() {
        let config = parse_config_parts(
            "[credentials]\nfile='nonexistent-endpoint-credentials.json'",
            "config.toml",
        )
        .unwrap()
        .settings;
        let empty = ProviderSource(Vec::new().into());
        let error = config.resolve_from(&empty).await.unwrap_err().to_string();
        assert!(error.contains("no BMC targets"));
    }

    #[tokio::test]
    async fn provider_and_static_identity_errors_use_the_same_mac_context() {
        for profile in ["missing", "bmc"] {
            let mut contents = format!(
                "[[targets]]\nip='192.0.2.1'\nmac='00:11:22:33:44:55'\ncredentials_profile='{profile}'\n"
            );
            if profile == "bmc" {
                contents.push_str(&contents.clone());
            }
            let contents = with_test_credentials(&contents);
            let config = parse_static_config(&contents, "config.toml").unwrap();
            let provider = ProviderSource(config.source.discover().await.unwrap());
            let provider_error = config
                .settings
                .resolve_from(&provider)
                .await
                .unwrap_err()
                .to_string();
            let static_error = parse_config(&contents, "config.toml")
                .unwrap_err()
                .to_string();
            assert_eq!(provider_error, static_error);
            assert!(
                provider_error.contains("(BMC 00:11:22:33:44:55):"),
                "{provider_error}"
            );
            let expected = if profile == "bmc" {
                "duplicates BMC MAC"
            } else {
                "unknown credentials_profile"
            };
            assert!(provider_error.contains(expected), "{provider_error}");
        }
    }

    #[tokio::test]
    async fn async_loading_roots_relative_credentials_at_the_config_directory() {
        let directory =
            std::env::temp_dir().join(format!("fleetint-endpoint-path-{}", std::process::id()));
        std::fs::create_dir_all(&directory).unwrap();
        let _cleanup = TestDirectoryCleanup(directory.clone());
        std::fs::write(
            directory.join("credentials.json"),
            r#"{"profiles":{"bmc":{"username":"user","password":"secret"}}}"#,
        )
        .unwrap();
        let path = directory.join("config.toml");
        std::fs::write(&path, "[credentials]\nfile='credentials.json'\n[[targets]]\nip='192.0.2.1'\nmac='00:11:22:33:44:55'\ncredentials_profile='bmc'").unwrap();
        let path = path.to_str().unwrap();
        let parsed = load_static_config(path).unwrap();
        assert!(parsed.settings.credentials_file.is_absolute());
        let provider = ProviderSource(parsed.source.discover().await.unwrap());
        for config in [
            load_config_async(path).await.unwrap(),
            parsed.settings.resolve_from(&provider).await.unwrap(),
        ] {
            assert_eq!(config.targets[0].username, "user");
            assert_eq!(config.targets[0].password, "secret");
            assert_eq!(config.credentials_file, directory.join("credentials.json"));
        }
    }

    #[tokio::test]
    async fn static_loaders_preserve_credentials_file_error_precedence() {
        for (index, fields) in [
            "mac='invalid'",
            "mac='00:11:22:33:44:55'\nname=' '",
            "mac='00:11:22:33:44:55'\nlabels={oob='false'}",
        ]
        .iter()
        .enumerate()
        {
            let input = format!(
                "[credentials]\nfile='missing-precedence-credentials.json'\n[[targets]]\nip='192.0.2.1'\n{fields}"
            );
            let path = write_config(&format!("endpoint-precedence-{index}"), &input);
            let _cleanup = TestFileCleanup(path.clone());
            let path = path.to_str().unwrap();
            let sync_error = load_config(path).unwrap_err().to_string();
            let async_error = load_config_async(path).await.unwrap_err().to_string();
            assert!(
                sync_error.contains("failed to read credentials.file"),
                "{sync_error}"
            );
            assert_eq!(async_error, sync_error);
        }
    }

    #[tokio::test]
    async fn nico_selection_roots_token_path_and_needs_no_static_targets_or_bmc_passwords_for_discovery()
     {
        let directory =
            std::env::temp_dir().join(format!("fleetint-nico-config-{}", std::process::id()));
        std::fs::create_dir_all(&directory).unwrap();
        let _cleanup = TestDirectoryCleanup(directory.clone());
        let path = directory.join("config.toml");
        let input = r#"
            compute_zone = "zone"
            [credentials]
            file = "missing-bmc-credentials.json"
            [endpoint_source]
            type = "nico"
            api_url = "https://nico.example.com/v2"
            org = "example"
            site_id = "site-id"
            token_file = "token"
            credentials_profile = "bmc"
        "#;
        std::fs::write(&path, input).unwrap();
        // An invalid header proves the rooted token file was read. No request is
        // sent and the nonexistent BMC credential file must not be consulted.
        std::fs::write(directory.join("token"), "invalid\nheader-value").unwrap();
        let path = path.to_str().unwrap();
        let parsed = parse_config_parts(input, path).unwrap();
        let SourceSelection::Nico(config) = parsed.source else {
            panic!("NICo source not selected")
        };
        assert_eq!(config.token_file, directory.join("token"));
        assert_eq!(config.api_name, "nico");
        let selected = load_source_config(path).await.unwrap();
        assert_eq!(
            selected.settings.defaults.compute_zone.as_deref(),
            Some("zone")
        );
        let discovery_error = selected.source.discover().await.unwrap_err().to_string();
        assert!(discovery_error.contains("invalid NICo bearer token header"));
        assert!(!discovery_error.contains("header-value"));
        let runtime_error = load_config_async(path).await.unwrap_err().to_string();
        assert_eq!(runtime_error, discovery_error);
        assert!(
            load_config(path)
                .unwrap_err()
                .to_string()
                .contains("supports static targets only")
        );
    }

    #[tokio::test]
    async fn source_selection_keeps_static_default_and_rejects_ambiguous_or_unknown_config() {
        let contents = with_test_credentials(
            "[[targets]]\nip='192.0.2.1'\nmac='00:11:22:33:44:55'\ncredentials_profile='bmc'",
        );
        let path = write_config("selected-static", &contents);
        let _cleanup = TestFileCleanup(path.clone());
        let selected = load_source_config(path.to_str().unwrap()).await.unwrap();
        let snapshot = selected.source.discover().await.unwrap();
        assert_eq!(snapshot.len(), 1);
        assert_eq!(snapshot[0].mac.to_string(), "00:11:22:33:44:55");
        let resolved = load_config_async(path.to_str().unwrap()).await.unwrap();
        assert_eq!(resolved.targets[0].username, "admin");
        let explicit_static = format!("{contents}\n[endpoint_source]\ntype='static'\n");
        std::fs::write(&path, explicit_static).unwrap();
        assert_eq!(
            load_config_async(path.to_str().unwrap())
                .await
                .unwrap()
                .targets
                .len(),
            1
        );
        let nico = "[credentials]\nfile='missing.json'\n[endpoint_source]\ntype='nico'\napi_url='https://nico.example.com/v2'\norg='example'\nsite_id='site-id'\ntoken_file='token'\n";
        let mixed = format!("{nico}\n[[targets]]\nip='192.0.2.1'\nmac='00:11:22:33:44:55'");
        assert!(
            parse_config_parts(&mixed, "config.toml")
                .err()
                .unwrap()
                .to_string()
                .contains("cannot be combined")
        );
        for invalid in [
            nico.replace("type='nico'", "type='netbox'"),
            nico.replace(
                "token_file='token'",
                "token_file='token'\npassword='private-value'",
            ),
            nico.replace("token_file='token'", "token_file=''"),
        ] {
            let error = parse_config_parts(&invalid, "config.toml")
                .err()
                .unwrap()
                .to_string();
            assert!(!error.contains("private-value"));
        }
        let invalid = nico.replace("https://nico.example.com/v2", "http://nico.example.com/v2");
        assert!(
            parse_config_parts(&invalid, "config.toml")
                .unwrap()
                .into_source()
                .err()
                .unwrap()
                .to_string()
                .contains("HTTPS")
        );
    }

    #[tokio::test]
    async fn runtime_fallback_is_bounded_scoped_and_credential_free() {
        let directory =
            std::env::temp_dir().join(format!("fleetint-nico-fallback-{}", std::process::id()));
        std::fs::create_dir_all(&directory).unwrap();
        let _cleanup = TestDirectoryCleanup(directory.clone());
        let path = directory.join("config.toml");
        std::fs::write(&path, "[credentials]\nfile='credentials.json'\n").unwrap();
        let credentials = directory.join("credentials.json");
        std::fs::write(
            &credentials,
            r#"{"profiles":{"bmc":{"username":"before","password":"fixture"}}}"#,
        )
        .unwrap();
        let settings = load_endpoint_config(path.to_str().unwrap()).unwrap();
        let key: NicoRestConfig = toml::from_str("api_url='https://nico.example.com/v2'\norg='example'\nsite_id='site'\ntoken_file='/private/token'\ncredentials_profile='bmc'\nmax_stale_seconds=60").unwrap();
        let mut endpoint = provider_endpoint();
        endpoint.source_reference = Some("machine-for-cache-test".into());
        let endpoints: crate::endpoint::EndpointSnapshot = vec![endpoint].into();
        let unavailable = || Err(crate::endpoint::nico::unavailable("NICo returned HTTP 401"));
        let mut loader = RuntimeConfigLoader::default();
        assert!(
            loader
                .resolve_discovery(&settings, key.clone(), unavailable())
                .await
                .is_err()
        );
        loader
            .resolve_discovery(&settings, key.clone(), Ok(endpoints.clone()))
            .await
            .unwrap();
        let first_time = loader.last_good.as_ref().unwrap().discovered_at;
        std::fs::write(
            &credentials,
            r#"{"profiles":{"bmc":{"username":"rotated","password":"new-fixture"}}}"#,
        )
        .unwrap();
        let fallback = loader
            .resolve_discovery(&settings, key.clone(), unavailable())
            .await
            .unwrap();
        assert_eq!(fallback.targets[0].username, "rotated");
        assert_eq!(loader.last_good.as_ref().unwrap().discovered_at, first_time);
        loader.last_good.as_mut().unwrap().discovered_at =
            std::time::Instant::now() - std::time::Duration::from_secs(61);
        assert!(
            loader
                .resolve_discovery(&settings, key.clone(), unavailable())
                .await
                .is_err()
        );
        assert!(loader.last_good.is_none());
        loader
            .resolve_discovery(&settings, key.clone(), Ok(endpoints.clone()))
            .await
            .unwrap();
        let mut other_site = key.clone();
        other_site.site_id = "another-site".into();
        assert!(
            loader
                .resolve_discovery(&settings, other_site, unavailable())
                .await
                .is_err()
        );
        loader
            .resolve_discovery(&settings, key.clone(), Ok(endpoints.clone()))
            .await
            .unwrap();
        assert!(
            loader
                .resolve_discovery(&settings, key.clone(), Ok(Vec::new().into()))
                .await
                .is_err()
        );
        assert!(loader.last_good.is_none());
        loader
            .resolve_discovery(&settings, key.clone(), Ok(endpoints.clone()))
            .await
            .unwrap();
        assert!(
            loader
                .resolve_discovery(&settings, key.clone(), Err("duplicate BMC MAC".into()))
                .await
                .is_err()
        );
        assert!(loader.last_good.is_none());
        loader
            .resolve_discovery(&settings, key.clone(), Ok(endpoints))
            .await
            .unwrap();
        std::fs::write(&credentials, r#"{"profiles":{}}"#).unwrap();
        let error = loader
            .resolve_discovery(&settings, key.clone(), unavailable())
            .await
            .unwrap_err()
            .to_string();
        assert!(error.contains("machine-for-cache-test"));
        std::fs::write(&credentials, "invalid JSON").unwrap();
        assert!(
            loader
                .resolve_discovery(&settings, key, unavailable())
                .await
                .is_err()
        );
    }

    struct TestDirectoryCleanup(PathBuf);

    impl Drop for TestDirectoryCleanup {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }

    struct TestFileCleanup(PathBuf);

    impl Drop for TestFileCleanup {
        fn drop(&mut self) {
            let _ = std::fs::remove_file(&self.0);
        }
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
