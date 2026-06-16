use std::collections::HashSet;
use std::error::Error;
use std::fs::File;
use std::io::{BufRead, BufReader};
use std::time::Duration;

#[path = "default.rs"]
pub mod defaults;

#[derive(Clone, Debug)]
pub struct Config {
    pub backend: BackendConfig,
    pub attestation: AttestationConfig,
    pub targets: Vec<Target>,
}

#[derive(Clone, Debug)]
pub struct BackendConfig {
    pub url: String,
    pub jwt: String,
    pub nonce_path: String,
    pub attestation_path: String,
}

#[derive(Clone, Debug)]
pub struct AttestationConfig {
    pub port: u16,
    pub insecure_tls: bool,
    pub concurrency: usize,
    pub redfish_timeout: Duration,
    pub task_timeout: Duration,
    pub task_poll_interval: Duration,
    pub interval: Option<Duration>,
    pub retry_interval: Duration,
    pub batch_timeout: Option<Duration>,
    pub measurement_indices: Vec<u8>,
}

#[derive(Clone, Debug)]
pub struct Target {
    pub name: String,
    pub node_uuid: String,
    pub address: String,
    pub username: String,
    pub password: String,
}

#[derive(Default)]
struct TargetBuilder {
    name: String,
    node_uuid: String,
    address: String,
    username: String,
    password: String,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum SectionKind {
    Backend,
    Attestation,
    Target,
}

pub fn load_config(path: &str) -> Result<Config, Box<dyn Error>> {
    let file = File::open(path)?;
    let reader = BufReader::new(file);
    let mut backend = BackendConfig {
        url: String::new(),
        jwt: String::new(),
        nonce_path: defaults::DEFAULT_BACKEND_NONCE_PATH.to_string(),
        attestation_path: defaults::DEFAULT_BACKEND_ATTESTATION_PATH.to_string(),
    };
    let mut attestation = AttestationConfig::default();
    let mut targets = Vec::new();
    let mut current_target: Option<TargetBuilder> = None;
    let mut section_kind: Option<SectionKind> = None;

    for (line_no, line) in reader.lines().enumerate() {
        let line_no = line_no + 1;
        let line = line?;
        let line = line.trim();
        if line.is_empty() || line.starts_with('#') || line.starts_with(';') {
            continue;
        }

        if let Some(section) = line
            .strip_prefix('[')
            .and_then(|line| line.strip_suffix(']'))
        {
            if let Some(target) = current_target.take() {
                targets.push(validate_target(target)?);
            }
            let section = section.trim();
            if section.is_empty() {
                return Err(format!("empty section name at line {line_no}").into());
            }
            match section.to_ascii_lowercase().as_str() {
                "backend" => {
                    section_kind = Some(SectionKind::Backend);
                }
                "attestation" => {
                    section_kind = Some(SectionKind::Attestation);
                }
                _ => {
                    section_kind = Some(SectionKind::Target);
                    current_target = Some(TargetBuilder {
                        name: section.to_string(),
                        ..TargetBuilder::default()
                    });
                }
            }
            continue;
        }

        let Some((key, value)) = line.split_once('=') else {
            return Err(format!("invalid config line {line_no}").into());
        };
        let key = key.trim().to_ascii_lowercase();
        let value = value.trim();
        match section_kind {
            Some(SectionKind::Backend) => parse_backend_key(&mut backend, &key, value, line_no)?,
            Some(SectionKind::Attestation) => {
                parse_attestation_key(&mut attestation, &key, value, line_no)?
            }
            Some(SectionKind::Target) => {
                let Some(target) = current_target.as_mut() else {
                    return Err(
                        format!("target key outside target section at line {line_no}").into(),
                    );
                };
                parse_target_key(target, &key, value, line_no)?;
            }
            None => return Err(format!("key outside section at line {line_no}").into()),
        }
    }

    if let Some(target) = current_target.take() {
        targets.push(validate_target(target)?);
    }
    validate_backend(&backend)?;
    if targets.is_empty() {
        return Err(format!("no attestation targets found in {path}").into());
    }
    validate_unique_targets(&targets)?;
    Ok(Config {
        backend,
        attestation,
        targets,
    })
}

impl Default for AttestationConfig {
    fn default() -> Self {
        defaults::attestation_config()
    }
}

fn parse_backend_key(
    backend: &mut BackendConfig,
    key: &str,
    value: &str,
    line_no: usize,
) -> Result<(), Box<dyn Error>> {
    match key {
        "url" | "backend_url" => backend.url = value.to_string(),
        "jwt" => backend.jwt = value.to_string(),
        "nonce_path" => backend.nonce_path = normalize_path(value),
        "attestation_path" => backend.attestation_path = normalize_path(value),
        other => return Err(format!("unknown backend key {other:?} at line {line_no}").into()),
    }
    Ok(())
}

fn parse_attestation_key(
    attestation: &mut AttestationConfig,
    key: &str,
    value: &str,
    line_no: usize,
) -> Result<(), Box<dyn Error>> {
    match key {
        "port" => attestation.port = parse_u16(value, key, line_no)?,
        "insecure_tls" => attestation.insecure_tls = parse_bool(value, key, line_no)?,
        "concurrency" => attestation.concurrency = parse_usize(value, key, line_no)?,
        "redfish_timeout_seconds" | "timeout_seconds" => {
            attestation.redfish_timeout = duration_secs(value, key, line_no)?
        }
        "task_timeout_seconds" => attestation.task_timeout = duration_secs(value, key, line_no)?,
        "task_poll_interval_millis" => {
            attestation.task_poll_interval = duration_millis(value, key, line_no)?
        }
        "interval_seconds" => {
            let duration = duration_secs(value, key, line_no)?;
            attestation.interval = if duration.is_zero() {
                None
            } else {
                Some(duration)
            };
        }
        "retry_interval_seconds" => {
            attestation.retry_interval = duration_secs(value, key, line_no)?
        }
        "batch_timeout_seconds" => {
            let duration = duration_secs(value, key, line_no)?;
            attestation.batch_timeout = if duration.is_zero() {
                None
            } else {
                Some(duration)
            };
        }
        "measurement_indices" => {
            attestation.measurement_indices = parse_measurement_indices(value, line_no)?
        }
        other => {
            return Err(format!("unknown attestation key {other:?} at line {line_no}").into());
        }
    }
    Ok(())
}

fn parse_target_key(
    target: &mut TargetBuilder,
    key: &str,
    value: &str,
    line_no: usize,
) -> Result<(), Box<dyn Error>> {
    match key {
        "node_uuid" => target.node_uuid = value.to_string(),
        "address" => target.address = value.to_string(),
        "username" => target.username = value.to_string(),
        "password" => target.password = value.to_string(),
        other => return Err(format!("unknown target key {other:?} at line {line_no}").into()),
    }
    Ok(())
}

fn validate_backend(backend: &BackendConfig) -> Result<(), Box<dyn Error>> {
    if backend.url.is_empty() {
        return Err("backend url is required".into());
    }
    if backend.jwt.is_empty() {
        return Err("backend jwt is required".into());
    }
    if backend.nonce_path.is_empty() {
        return Err("backend nonce_path is required".into());
    }
    if !backend.nonce_path.contains("{node_uuid}") {
        return Err("backend nonce_path must contain {node_uuid}".into());
    }
    if backend.attestation_path.is_empty() {
        return Err("backend attestation_path is required".into());
    }
    Ok(())
}

fn validate_target(target: TargetBuilder) -> Result<Target, Box<dyn Error>> {
    if target.node_uuid.is_empty() {
        return Err(format!("target {:?} node_uuid is required", target.name).into());
    }
    if target.address.is_empty() {
        return Err(format!("target {:?} address is required", target.name).into());
    }
    if target.username.is_empty() {
        return Err(format!("target {:?} username is required", target.name).into());
    }
    if target.password.is_empty() {
        return Err(format!("target {:?} password is required", target.name).into());
    }
    Ok(Target {
        name: target.name,
        node_uuid: target.node_uuid,
        address: target.address,
        username: target.username,
        password: target.password,
    })
}

fn validate_unique_targets(targets: &[Target]) -> Result<(), Box<dyn Error>> {
    let mut names = HashSet::new();
    let mut node_uuids = HashSet::new();
    for target in targets {
        if !names.insert(target.name.as_str()) {
            return Err(format!("duplicate target name {:?}", target.name).into());
        }
        if !node_uuids.insert(target.node_uuid.as_str()) {
            return Err(format!("duplicate node_uuid {:?}", target.node_uuid).into());
        }
    }
    Ok(())
}

fn normalize_path(value: &str) -> String {
    if value.starts_with('/') {
        value.to_string()
    } else {
        format!("/{value}")
    }
}

fn parse_bool(value: &str, key: &str, line_no: usize) -> Result<bool, Box<dyn Error>> {
    match value.to_ascii_lowercase().as_str() {
        "true" | "yes" | "1" => Ok(true),
        "false" | "no" | "0" => Ok(false),
        _ => Err(format!("invalid boolean for {key:?} at line {line_no}").into()),
    }
}

fn parse_u16(value: &str, key: &str, line_no: usize) -> Result<u16, Box<dyn Error>> {
    value
        .parse()
        .map_err(|err| format!("invalid integer for {key:?} at line {line_no}: {err}").into())
}

fn parse_usize(value: &str, key: &str, line_no: usize) -> Result<usize, Box<dyn Error>> {
    value
        .parse()
        .map_err(|err| format!("invalid integer for {key:?} at line {line_no}: {err}").into())
}

fn parse_u8_range(
    value: &str,
    key: &str,
    line_no: usize,
    min: u8,
    max: u8,
) -> Result<u8, Box<dyn Error>> {
    let parsed: u8 = value
        .parse()
        .map_err(|err| format!("invalid integer for {key:?} at line {line_no}: {err}"))?;
    if parsed < min || parsed > max {
        return Err(format!("{key:?} at line {line_no} must be between {min} and {max}").into());
    }
    Ok(parsed)
}

fn duration_secs(value: &str, key: &str, line_no: usize) -> Result<Duration, Box<dyn Error>> {
    Ok(Duration::from_secs(parse_usize(value, key, line_no)? as u64))
}

fn duration_millis(value: &str, key: &str, line_no: usize) -> Result<Duration, Box<dyn Error>> {
    Ok(Duration::from_millis(
        parse_usize(value, key, line_no)? as u64
    ))
}

fn parse_measurement_indices(value: &str, line_no: usize) -> Result<Vec<u8>, Box<dyn Error>> {
    let mut out = Vec::new();
    for item in value.split(',') {
        let item = item.trim();
        if item.is_empty() {
            continue;
        }
        out.push(parse_u8_range(
            item,
            "measurement_indices",
            line_no,
            0,
            255,
        )?);
    }
    if out.is_empty() {
        return Err(format!("measurement_indices at line {line_no} cannot be empty").into());
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    #[test]
    fn loads_backend_attestation_and_targets() {
        let path = write_config(
            "valid",
            r#"
            [backend]
            url = https://backend.example.com
            jwt = token
            nonce_path = v1/oob/nodes/{node_uuid}/nonce
            attestation_path = v1/oob/attestations

            [attestation]
            concurrency = 32
            insecure_tls = true
            measurement_indices = 1, 2, 255

            [node-a]
            node_uuid = uuid-a
            address = 10.0.0.1
            username = admin
            password = secret

            [node-b]
            node_uuid = uuid-b
            address = https://bmc.example.com
            username = root
            password = pass
            "#,
        );

        let config = load_config(path.to_str().expect("path")).expect("config");

        assert_eq!(config.backend.url, "https://backend.example.com");
        assert_eq!(config.backend.nonce_path, "/v1/oob/nodes/{node_uuid}/nonce");
        assert_eq!(config.backend.attestation_path, "/v1/oob/attestations");
        assert!(config.attestation.insecure_tls);
        assert_eq!(config.attestation.concurrency, 32);
        assert_eq!(config.attestation.measurement_indices, vec![1, 2, 255]);
        assert_eq!(config.targets.len(), 2);
        assert_eq!(config.targets[0].name, "node-a");
        assert_eq!(config.targets[0].node_uuid, "uuid-a");
    }

    #[test]
    fn rejects_missing_node_uuid() {
        let path = write_config(
            "missing-node-uuid",
            r#"
            [backend]
            url = https://backend.example.com
            jwt = token

            [node-a]
            address = 10.0.0.1
            username = admin
            password = secret
            "#,
        );

        let error = load_config(path.to_str().expect("path"))
            .expect_err("missing node_uuid should fail")
            .to_string();

        assert!(error.contains("node_uuid is required"));
    }

    #[test]
    fn rejects_missing_backend() {
        let path = write_config(
            "missing-backend",
            r#"
            [node-a]
            node_uuid = uuid-a
            address = 10.0.0.1
            username = admin
            password = secret
            "#,
        );

        let error = load_config(path.to_str().expect("path"))
            .expect_err("missing backend should fail")
            .to_string();

        assert!(error.contains("backend url is required"));
    }

    #[test]
    fn rejects_nonce_path_without_node_uuid_placeholder() {
        let path = write_config(
            "bad-nonce-path",
            r#"
            [backend]
            url = https://backend.example.com
            jwt = token
            nonce_path = /v1/oob/nonce

            [node-a]
            node_uuid = uuid-a
            address = 10.0.0.1
            username = admin
            password = secret
            "#,
        );

        let error = load_config(path.to_str().expect("path"))
            .expect_err("nonce path without placeholder should fail")
            .to_string();

        assert!(error.contains("nonce_path must contain {node_uuid}"));
    }

    #[test]
    fn rejects_duplicate_node_uuid() {
        let path = write_config(
            "duplicate-node-uuid",
            r#"
            [backend]
            url = https://backend.example.com
            jwt = token

            [node-a]
            node_uuid = node-1
            address = 10.0.0.1
            username = admin
            password = secret

            [node-b]
            node_uuid = node-1
            address = 10.0.0.2
            username = admin
            password = secret
            "#,
        );

        let error = load_config(path.to_str().expect("path"))
            .expect_err("duplicate node UUID should fail")
            .to_string();

        assert!(error.contains("duplicate node_uuid"));
    }

    fn write_config(name: &str, content: &str) -> std::path::PathBuf {
        let mut path = std::env::temp_dir();
        path.push(format!(
            "fleet-intelligence-oob-attestation-{name}-{}.config",
            std::process::id()
        ));
        fs::write(&path, unindent(content)).expect("write config");
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
