use std::collections::{BTreeMap, HashSet};
use std::error::Error;
use std::fmt;
use std::net::IpAddr;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

pub use crate::models::TargetKind;

const MAX_NICO_LABELS: usize = 32;
const MAX_NICO_LABEL_VALUE_BYTES: usize = 1024;
const OOB_LABEL_NAME: &str = "oob";
const OOB_LABEL_VALUE: &str = "true";
const MACHINE_NAME_LABEL_NAME: &str = "machine_name";
const COMPUTE_ZONE_LABEL_NAME: &str = "compute_zone";
const NODE_GROUP_LABEL_NAME: &str = "node_group";
pub const DEFAULT_NMX_C_PORT: u16 = 9370;
const RESERVED_NICO_LABELS: &[&str] = &[
    "collector_type",
    "endpoint_ip",
    "endpoint_key",
    "endpoint_mac",
    "machine_id",
    "machine_slot_number",
    "machine_tray_index",
    MACHINE_NAME_LABEL_NAME,
    COMPUTE_ZONE_LABEL_NAME,
    NODE_GROUP_LABEL_NAME,
    "nvlink_domain_uuid",
    OOB_LABEL_NAME,
    "rack_id",
    "serial_number",
    "switch_id",
    "switch_slot_number",
    "switch_tray_index",
    "system_uuid",
];

#[derive(Clone, Debug)]
pub struct CollectorConfig {
    pub racks: Vec<Rack>,
    pub targets: Vec<Target>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Rack {
    pub id: String,
    pub nmxc: Option<NmxcConfig>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct NmxcConfig {
    pub ip: IpAddr,
    pub port: u16,
    pub ca_file: Option<PathBuf>,
    pub client_cert_file: Option<PathBuf>,
    pub client_key_file: Option<PathBuf>,
    pub server_name: Option<String>,
}

impl NmxcConfig {
    pub fn uses_tls(&self) -> bool {
        self.ca_file.is_some()
            || self.client_cert_file.is_some()
            || self.client_key_file.is_some()
            || self.server_name.is_some()
    }

    pub fn endpoint_url(&self) -> String {
        let scheme = if self.uses_tls() { "https" } else { "http" };
        match self.ip {
            IpAddr::V4(ip) => format!("{scheme}://{ip}:{}", self.port),
            IpAddr::V6(ip) => format!("{scheme}://[{ip}]:{}", self.port),
        }
    }
}

#[derive(Clone)]
pub struct Target {
    pub kind: TargetKind,
    pub name: Option<String>,
    pub compute_zone: Option<String>,
    pub node_group: Option<String>,
    pub rack_id: Option<String>,
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
            .field("kind", &self.kind)
            .field("name", &self.name)
            .field("compute_zone", &self.compute_zone)
            .field("node_group", &self.node_group)
            .field("rack_id", &self.rack_id)
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
    #[serde(default)]
    credentials: Option<FileCredentialConfig>,
    #[serde(default)]
    racks: Vec<RawRackConfig>,
    #[serde(default)]
    machines: Vec<RawTargetConfig>,
    #[serde(default)]
    targets: Vec<RawTargetConfig>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct RawRackConfig {
    id: String,
    #[serde(default)]
    nmxc: Option<RawNmxcConfig>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct RawNmxcConfig {
    ip: IpAddr,
    #[serde(default = "default_nmxc_port")]
    port: u16,
    #[serde(default)]
    ca_file: Option<PathBuf>,
    #[serde(default)]
    client_cert_file: Option<PathBuf>,
    #[serde(default)]
    client_key_file: Option<PathBuf>,
    #[serde(default)]
    server_name: Option<String>,
}

fn default_nmxc_port() -> u16 {
    DEFAULT_NMX_C_PORT
}

/// Paths containing a complete BMC username and password pair.
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct FileCredentialConfig {
    #[serde(default)]
    username_file: Option<PathBuf>,
    #[serde(default)]
    password_file: Option<PathBuf>,
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
    #[serde(default)]
    rack_id: Option<String>,
    ip: IpAddr,
    #[serde(default)]
    port: Option<u16>,
    #[serde(default)]
    mac: Option<String>,
    #[serde(default)]
    username: Option<String>,
    #[serde(default)]
    password: Option<String>,
    #[serde(default)]
    username_file: Option<PathBuf>,
    #[serde(default)]
    password_file: Option<PathBuf>,
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

    if config.machines.is_empty() && config.targets.is_empty() && config.racks.is_empty() {
        return Err(format!(
            "no racks or BMC targets found in {path}; configure [[racks]] or [[targets]] (legacy [[machines]] is also accepted)"
        )
        .into());
    }

    validate_nico_labels("labels", &config.labels)?;
    let default_compute_zone = normalize_config_value("compute_zone", config.compute_zone)?;
    let default_node_group = normalize_config_value("node_group", config.node_group)?;
    let config_dir = Path::new(path).parent().unwrap_or_else(|| Path::new("."));
    let mut rack_ids = HashSet::new();
    let mut racks = Vec::with_capacity(config.racks.len());
    for (index, rack) in config.racks.into_iter().enumerate() {
        let rack = rack.into_rack(index, config_dir)?;
        if !rack_ids.insert(rack.id.clone()) {
            return Err(format!("duplicate rack id {:?}", rack.id).into());
        }
        racks.push(rack);
    }
    let default_credentials = config
        .credentials
        .as_ref()
        .map(|credentials| credentials.resolve("credentials", config_dir))
        .transpose()?;

    let target_count = config.machines.len() + config.targets.len();
    let mut configured_targets = Vec::with_capacity(target_count);
    for (index, machine) in config.machines.into_iter().enumerate() {
        if machine.kind.is_some() {
            return Err(format!(
                "machines[{index}].kind is not supported; use [[targets]] to configure a target kind"
            )
            .into());
        }
        configured_targets.push((machine, TargetKind::Compute, format!("machines[{index}]")));
    }
    for (index, target) in config.targets.into_iter().enumerate() {
        let kind = target.kind.unwrap_or_default();
        configured_targets.push((target, kind, format!("targets[{index}]")));
    }

    let mut macs = HashSet::new();
    let mut targets = Vec::with_capacity(target_count);
    for (configured_target, kind, context) in configured_targets {
        let mut target = configured_target.into_target(
            kind,
            &context,
            default_credentials.as_ref(),
            config_dir,
            default_compute_zone.as_deref(),
            default_node_group.as_deref(),
        )?;
        let mut labels = config.labels.clone();
        labels.extend(target.labels);
        validate_nico_labels(&format!("{context}.labels"), &labels)?;
        let rack_has_nmxc = match &target.rack_id {
            Some(rack_id) if !rack_ids.contains(rack_id) => {
                return Err(format!(
                    "{context}.rack_id {rack_id:?} does not match a configured rack"
                )
                .into());
            }
            Some(rack_id) => racks
                .iter()
                .find(|rack| &rack.id == rack_id)
                .is_some_and(|rack| rack.nmxc.is_some()),
            None => false,
        };
        let internal_label_count = 1
            + usize::from(target.name.is_some())
            + usize::from(target.compute_zone.is_some())
            + usize::from(target.node_group.is_some())
            + usize::from(target.rack_id.is_some())
            + usize::from(rack_has_nmxc);
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
        if let Some(rack_id) = &target.rack_id {
            labels.insert("rack_id".to_string(), rack_id.clone());
        }
        target.labels = labels;
        if !macs.insert(target.mac.to_ascii_lowercase()) {
            return Err(format!("duplicate BMC MAC {:?}", target.mac).into());
        }
        targets.push(target);
    }

    Ok(CollectorConfig { racks, targets })
}

impl RawRackConfig {
    fn into_rack(self, index: usize, config_dir: &Path) -> Result<Rack, Box<dyn Error>> {
        let context = format!("racks[{index}]");
        let id = normalize_config_value(&format!("{context}.id"), Some(self.id))?
            .expect("rack id is present");
        let nmxc = self
            .nmxc
            .map(|nmxc| nmxc.resolve(&format!("{context}.nmxc"), config_dir))
            .transpose()?;
        Ok(Rack { id, nmxc })
    }
}

impl RawNmxcConfig {
    fn resolve(self, context: &str, config_dir: &Path) -> Result<NmxcConfig, Box<dyn Error>> {
        if self.port == 0 {
            return Err(format!("{context}.port must be greater than zero").into());
        }
        if self.client_cert_file.is_some() != self.client_key_file.is_some() {
            return Err(format!(
                "{context}.client_cert_file and {context}.client_key_file must be configured together"
            )
            .into());
        }
        let server_name =
            normalize_config_value(&format!("{context}.server_name"), self.server_name)?;
        Ok(NmxcConfig {
            ip: self.ip,
            port: self.port,
            ca_file: resolve_optional_path(self.ca_file, config_dir),
            client_cert_file: resolve_optional_path(self.client_cert_file, config_dir),
            client_key_file: resolve_optional_path(self.client_key_file, config_dir),
            server_name,
        })
    }
}

fn resolve_optional_path(path: Option<PathBuf>, config_dir: &Path) -> Option<PathBuf> {
    path.map(|path| {
        if path.is_absolute() {
            path
        } else {
            config_dir.join(path)
        }
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
        default_credentials: Option<&ResolvedCredentials>,
        config_dir: &Path,
        default_compute_zone: Option<&str>,
        default_node_group: Option<&str>,
    ) -> Result<Target, Box<dyn Error>> {
        let credentials = self.resolve_credentials(context, default_credentials, config_dir)?;
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
        let rack_id = normalize_config_value(&format!("{context}.rack_id"), self.rack_id)?;
        let mac = match self.mac {
            Some(mac) if mac.trim().is_empty() => {
                return Err(format!("{context}.mac cannot be blank").into());
            }
            Some(mac) => mac,
            None => derive_mac_from_ip(self.ip),
        };
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
            rack_id,
            ip: self.ip,
            port: self.port,
            mac,
            address,
            username: credentials.username,
            password: credentials.password,
            labels: self.labels,
        })
    }

    /// Resolve a complete machine override or inherit the default credential pair.
    fn resolve_credentials(
        &self,
        context: &str,
        default_credentials: Option<&ResolvedCredentials>,
        config_dir: &Path,
    ) -> Result<ResolvedCredentials, Box<dyn Error>> {
        let has_inline = self.username.is_some() || self.password.is_some();
        let has_files = self.username_file.is_some() || self.password_file.is_some();

        if has_inline && has_files {
            return Err(format!(
                "{context} must use either username/password or username_file/password_file, not both"
            )
            .into());
        }
        if has_inline {
            return ResolvedCredentials::from_inline(
                self.username.as_deref(),
                self.password.as_deref(),
                context,
            );
        }
        if has_files {
            return FileCredentialConfig {
                username_file: self.username_file.clone(),
                password_file: self.password_file.clone(),
            }
            .resolve(context, config_dir);
        }

        default_credentials.cloned().ok_or_else(|| {
            format!("{context} requires credentials or a top-level [credentials] file pair").into()
        })
    }
}

/// BMC credentials resolved from inline values or mounted files.
#[derive(Clone)]
struct ResolvedCredentials {
    username: String,
    password: String,
}

impl ResolvedCredentials {
    /// Validate and materialize a complete inline credential pair.
    fn from_inline(
        username: Option<&str>,
        password: Option<&str>,
        context: &str,
    ) -> Result<Self, Box<dyn Error>> {
        let username = username.ok_or_else(|| format!("{context}.username is required"))?;
        let password = password.ok_or_else(|| format!("{context}.password is required"))?;
        validate_credentials(username, password, context)
    }
}

impl FileCredentialConfig {
    /// Read and validate a complete credential pair from its configured files.
    fn resolve(
        &self,
        context: &str,
        config_dir: &Path,
    ) -> Result<ResolvedCredentials, Box<dyn Error>> {
        let username_path = self
            .username_file
            .as_deref()
            .ok_or_else(|| format!("{context}.username_file is required"))?;
        let password_path = self
            .password_file
            .as_deref()
            .ok_or_else(|| format!("{context}.password_file is required"))?;
        let username = read_credential_file(username_path, config_dir, context, "username_file")?;
        let password = read_credential_file(password_path, config_dir, context, "password_file")?;
        validate_credentials(&username, &password, context)
    }
}

/// Read one credential file, resolving relative paths from the config directory.
fn read_credential_file(
    path: &Path,
    config_dir: &Path,
    context: &str,
    field: &str,
) -> Result<String, Box<dyn Error>> {
    let resolved_path = if path.is_absolute() {
        path.to_path_buf()
    } else {
        config_dir.join(path)
    };
    let value = std::fs::read_to_string(&resolved_path).map_err(|error| {
        format!(
            "failed to read {context}.{field} {}: {error}",
            resolved_path.display()
        )
    })?;
    Ok(strip_one_line_ending(value))
}

/// Remove one conventional trailing line ending while preserving credential data.
fn strip_one_line_ending(mut value: String) -> String {
    if value.ends_with("\r\n") {
        value.truncate(value.len() - 2);
    } else if value.ends_with('\n') {
        value.pop();
    }
    value
}

/// Validate resolved values without exposing either credential in errors.
fn validate_credentials(
    username: &str,
    password: &str,
    context: &str,
) -> Result<ResolvedCredentials, Box<dyn Error>> {
    if username.trim().is_empty() {
        return Err(format!("{context}.username is required").into());
    }
    if password.is_empty() || password.chars().all(char::is_whitespace) {
        return Err(format!("{context}.password is required by the OOB collector").into());
    }
    Ok(ResolvedCredentials {
        username: username.to_string(),
        password: password.to_string(),
    })
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
    render_nico_config_with_domains(config, options, &BTreeMap::new())
}

pub fn render_nico_config_with_domains(
    config: &CollectorConfig,
    options: &NicoConfigOptions,
    domain_uuids: &BTreeMap<String, String>,
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
            if let Some(domain_uuid) = target
                .rack_id
                .as_ref()
                .and_then(|rack_id| domain_uuids.get(rack_id))
            {
                labels.insert("nvlink_domain_uuid".to_string(), domain_uuid.clone());
            }
            NicoStaticBmcEndpoint {
                ip: target.ip,
                port: target.port,
                mac: &target.mac,
                username: &target.username,
                password: &target.password,
                labels,
                unknown: (target.kind == TargetKind::Unknown).then_some(NicoEndpointKind {}),
                machine: (target.kind == TargetKind::Compute).then_some(NicoEndpointKind {}),
                switch: (target.kind == TargetKind::NvSwitch).then_some(NicoEndpointKind {}),
                power_shelf: (target.kind == TargetKind::PowerShelf).then_some(NicoEndpointKind {}),
            }
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
                // "auto" streams BMC log entries over Redfish SSE and downgrades
                // to LogService polling per endpoint when /EventService is absent.
                mode: "auto",
            },
        },
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
                    include_diagnostics: false,
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
    use super::*;

    #[test]
    fn parses_rack_nmxc_and_defaults_plaintext_port() {
        let config = parse_config(
            r#"
            [[racks]]
            id = "rack-12"

            [racks.nmxc]
            ip = "192.0.2.12"
            "#,
            "test.toml",
        )
        .expect("rack-only NMX-C config");

        let nmxc = config.racks[0].nmxc.as_ref().expect("NMX-C config");
        assert_eq!(nmxc.port, DEFAULT_NMX_C_PORT);
        assert_eq!(nmxc.endpoint_url(), "http://192.0.2.12:9370");
    }

    #[test]
    fn resolves_nmxc_tls_paths_and_requires_client_cert_pair() {
        let config = parse_config(
            r#"
            [[racks]]
            id = "rack-12"

            [racks.nmxc]
            ip = "192.0.2.12"
            ca_file = "secrets/ca.pem"
            client_cert_file = "secrets/tls.crt"
            client_key_file = "secrets/tls.key"
            server_name = "nmxc.example.com"
            "#,
            "/etc/fleetint/config.toml",
        )
        .expect("TLS config");
        let nmxc = config.racks[0].nmxc.as_ref().expect("NMX-C config");
        assert_eq!(nmxc.endpoint_url(), "https://192.0.2.12:9370");
        assert_eq!(
            nmxc.ca_file,
            Some(PathBuf::from("/etc/fleetint/secrets/ca.pem"))
        );

        let error = parse_config(
            r#"
            [[racks]]
            id = "rack-12"

            [racks.nmxc]
            ip = "192.0.2.12"
            client_cert_file = "tls.crt"
            "#,
            "test.toml",
        )
        .expect_err("partial mTLS config must fail")
        .to_string();
        assert!(error.contains("must be configured together"));
    }

    #[test]
    fn target_rack_reference_must_exist() {
        let error = parse_config(
            r#"
            [[targets]]
            kind = "compute"
            rack_id = "missing-rack"
            ip = "10.0.0.1"
            username = "admin"
            password = "secret"
            "#,
            "test.toml",
        )
        .expect_err("unknown rack must fail")
        .to_string();
        assert!(error.contains("does not match a configured rack"));
    }

    #[test]
    fn parses_target_kinds_and_normalizes_legacy_machines_to_compute() {
        let config = parse_config(
            r#"
            [[machines]]
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
        .expect("config");

        assert_eq!(
            config
                .targets
                .iter()
                .map(|target| target.kind)
                .collect::<Vec<_>>(),
            vec![
                TargetKind::Compute,
                TargetKind::Compute,
                TargetKind::NvSwitch,
                TargetKind::PowerShelf,
            ]
        );
    }

    #[test]
    fn omitted_target_kind_defaults_to_unknown_and_legacy_machines_cannot_set_kind() {
        let config = parse_config(
            r#"
            [[targets]]
            ip = "10.0.0.1"
            username = "admin"
            password = "secret"
            "#,
            "test.toml",
        )
        .expect("target kind should be optional");
        assert_eq!(config.targets[0].kind, TargetKind::Unknown);

        let legacy = parse_config(
            r#"
            [[machines]]
            kind = "nvswitch"
            ip = "10.0.0.1"
            username = "admin"
            password = "secret"
            "#,
            "test.toml",
        )
        .expect_err("legacy machine kind should be rejected")
        .to_string();
        assert!(legacy.contains("use [[targets]]"));
    }

    #[test]
    fn applies_shared_placement_and_machine_label_overrides() {
        let path = write_config(
            "valid",
            r#"
            compute_zone = "us-west-dc1"
            node_group = "gb200-training"

            [labels]
            site = "shared"

            [[machines]]
            name = "node-01"
            compute_zone = "us-west-dc2"
            ip = "10.0.0.1"
            port = 8443
            mac = "00:11:22:33:44:55"
            username = "admin"
            password = "secret"
            labels = { site = "dev", environment = "development" }

            [[machines]]
            ip = "2001:db8::1"
            mac = "00:11:22:33:44:66"
            username = "root"
            password = "pass"
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
        assert_eq!(config.targets[0].mac, "00:11:22:33:44:55");
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
        assert_eq!(config.targets[1].mac, "00:11:22:33:44:66");
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
    fn applies_top_level_file_credentials_and_machine_override() {
        let default_username = write_credential("default-username", "default-admin\n");
        let default_password = write_credential("default-password", "default-secret\n");
        let override_username = write_credential("override-username", "node-admin\r\n");
        let override_password = write_credential("override-password", "node-secret\r\n");
        let path = write_config(
            "file-credentials",
            &format!(
                r#"
                [credentials]
                username_file = "{}"
                password_file = "{}"

                [[machines]]
                ip = "10.0.0.1"

                [[machines]]
                ip = "10.0.0.2"
                username_file = "{}"
                password_file = "{}"
                "#,
                default_username.display(),
                default_password.display(),
                override_username.display(),
                override_password.display(),
            ),
        );

        let config = load_config(path.to_str().expect("path")).expect("config");

        assert_eq!(config.targets[0].username, "default-admin");
        assert_eq!(config.targets[0].password, "default-secret");
        assert_eq!(config.targets[1].username, "node-admin");
        assert_eq!(config.targets[1].password, "node-secret");
    }

    #[test]
    fn resolves_relative_credential_paths_from_config_directory() {
        let mut directory = std::env::temp_dir();
        directory.push(format!(
            "fleet-intelligence-oob-relative-credentials-{}",
            std::process::id()
        ));
        std::fs::create_dir_all(directory.join("secrets")).expect("credential directory");
        std::fs::write(directory.join("secrets/username"), "admin\n").expect("username");
        std::fs::write(directory.join("secrets/password"), "secret\n").expect("password");
        let config_path = directory.join("config.toml");
        std::fs::write(
            &config_path,
            unindent(
                r#"
                [credentials]
                username_file = "secrets/username"
                password_file = "secrets/password"

                [[machines]]
                ip = "10.0.0.1"
                "#,
            ),
        )
        .expect("config");

        let config = load_config(config_path.to_str().expect("path")).expect("config");

        assert_eq!(config.targets[0].username, "admin");
        assert_eq!(config.targets[0].password, "secret");
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
    fn trims_named_fields_and_rejects_blank_values() {
        let config = parse_config(
            r#"
            compute_zone = "  us-west-dc1  "
            node_group = "  shared-group  "

            [[machines]]
            name = "  node-01  "
            node_group = "  gb200-training  "
            ip = "10.0.0.10"
            username = "admin"
            password = "secret"
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

        let error = parse_config(
            r#"
            [[machines]]
            name = "   "
            ip = "10.0.0.10"
            username = "admin"
            password = "secret"
            "#,
            "test.toml",
        )
        .expect_err("blank name should fail")
        .to_string();
        assert!(error.contains("name cannot be blank"));

        for field in ["compute_zone", "node_group"] {
            let error = parse_config(
                &format!(
                    r#"
                    {field} = "   "

                    [[machines]]
                    ip = "10.0.0.10"
                    username = "admin"
                    password = "secret"
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
        let username = write_credential("example-username", "admin\n");
        let password = write_credential("example-password", "secret\n");
        let override_username =
            write_credential("example-switch-tray-01-username", "switch-admin\n");
        let override_password =
            write_credential("example-switch-tray-01-password", "switch-secret\n");
        let contents = include_str!("../../../config.example.toml")
            .replace(
                "/run/secrets/bmc/default/username",
                username.to_str().expect("username path"),
            )
            .replace(
                "/run/secrets/bmc/default/password",
                password.to_str().expect("password path"),
            )
            .replace(
                "/run/secrets/bmc/switch-tray-01/username",
                override_username.to_str().expect("override username path"),
            )
            .replace(
                "/run/secrets/bmc/switch-tray-01/password",
                override_password.to_str().expect("override password path"),
            );
        let config = parse_config(&contents, "config.example.toml").expect("OOB config example");

        assert_eq!(config.racks.len(), 1);
        assert_eq!(config.racks[0].id, "rack-12");
        assert_eq!(
            config.racks[0].nmxc.as_ref().map(NmxcConfig::endpoint_url),
            Some("http://192.0.2.12:9370".to_string())
        );
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
        assert_eq!(config.targets[0].mac, "aa:bb:cc:dd:ee:ff");
        assert_eq!(config.targets[0].rack_id.as_deref(), Some("rack-12"));
        assert_eq!(
            config.targets[0].labels.get("compute_zone"),
            Some(&"us-west-dc1".to_string())
        );
        assert_eq!(
            config.targets[0].labels.get("node_group"),
            Some(&"gb200-training".to_string())
        );
        assert_eq!(
            config.targets[0].labels.get("rack_id"),
            Some(&"rack-12".to_string())
        );
        assert_eq!(config.targets[1].kind, TargetKind::NvSwitch);
        assert_eq!(config.targets[1].name.as_deref(), Some("switch-tray-01"));
        assert_eq!(config.targets[1].mac, "02:00:0a:00:00:0b");
        assert_eq!(config.targets[1].username, "switch-admin");
        assert_eq!(config.targets[2].kind, TargetKind::PowerShelf);
        assert_eq!(config.targets[2].name.as_deref(), Some("power-shelf-01"));
        assert_eq!(config.targets[2].mac, "02:00:0a:00:00:0c");
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

        assert!(error.contains("machines[0].password is required"));
    }

    #[test]
    fn rejects_partial_and_mixed_credential_sources() {
        for (name, content, expected) in [
            (
                "partial-default",
                r#"
                [credentials]
                username_file = "/run/secrets/bmc/username"

                [[machines]]
                ip = "10.0.0.1"
                "#,
                "credentials.password_file is required",
            ),
            (
                "partial-machine-files",
                r#"
                [[machines]]
                ip = "10.0.0.1"
                username_file = "/run/secrets/bmc/username"
                "#,
                "machines[0].password_file is required",
            ),
            (
                "mixed-machine",
                r#"
                [[machines]]
                ip = "10.0.0.1"
                username = "admin"
                password_file = "/run/secrets/bmc/password"
                "#,
                "must use either username/password or username_file/password_file",
            ),
        ] {
            let path = write_config(name, content);
            let error = load_config(path.to_str().expect("path"))
                .expect_err("invalid credentials should fail")
                .to_string();
            assert!(error.contains(expected), "unexpected error: {error}");
        }
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
    }

    #[test]
    fn renders_discovered_nvlink_domain_uuid_on_targets_in_the_rack() {
        let collector = parse_config(
            r#"
            [[racks]]
            id = "rack-12"

            [racks.nmxc]
            ip = "192.0.2.12"

            [[targets]]
            kind = "compute"
            rack_id = "rack-12"
            ip = "10.0.0.1"
            username = "admin"
            password = "secret"
            "#,
            "test.toml",
        )
        .expect("collector config");
        let domain_uuids = BTreeMap::from([(
            "rack-12".to_string(),
            "c2a9196f-9264-4c42-8298-09d2d1dc14d5".to_string(),
        )]);
        let rendered = render_nico_config_with_domains(
            &collector,
            &NicoConfigOptions::default(),
            &domain_uuids,
        )
        .expect("NICo config");
        let rendered: toml::Value = toml::from_str(&rendered).expect("rendered TOML");
        let labels = &rendered["endpoint_sources"]["static_bmc_endpoints"][0]["labels"];

        assert_eq!(labels["rack_id"].as_str(), Some("rack-12"));
        assert_eq!(
            labels["nvlink_domain_uuid"].as_str(),
            Some("c2a9196f-9264-4c42-8298-09d2d1dc14d5")
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
        assert_eq!(
            rendered["collectors"]["logs"]["mode"].as_str(),
            Some("auto")
        );
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

    #[test]
    fn rejects_labels_that_override_internal_labels() {
        for label in [
            "compute_zone",
            "machine_name",
            "node_group",
            "oob",
            "system_uuid",
        ] {
            let error = parse_config(
                &format!(
                    r#"
                    labels = {{ {label} = "override" }}

                    [[machines]]
                    ip = "10.0.0.1"
                    mac = "00:11:22:33:44:55"
                    username = "admin"
                    password = "secret"
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
