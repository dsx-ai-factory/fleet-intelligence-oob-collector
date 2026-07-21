use std::collections::BTreeMap;
use std::error::Error;
use std::fs::OpenOptions;
use std::io::Write;
#[cfg(unix)]
use std::os::unix::fs::OpenOptionsExt;
use std::path::{Path, PathBuf};

use serde::Serialize;

use crate::config::Target;
use crate::models::NodeInventory;

use super::InventorySink;

/// Writes the file-inventory contract consumed by NICo health cluster discovery.
pub struct NicoHealthSink {
    output_path: PathBuf,
    targets: Vec<Target>,
}

impl NicoHealthSink {
    pub fn new(output_path: impl Into<PathBuf>, targets: Vec<Target>) -> Self {
        Self {
            output_path: output_path.into(),
            targets,
        }
    }
}

impl InventorySink for NicoHealthSink {
    fn write(&self, inventories: &[NodeInventory]) -> Result<(), Box<dyn Error>> {
        let rendered = render_json(&self.targets, inventories)?;
        write_atomically(&self.output_path, rendered.as_bytes())?;
        Ok(())
    }
}

#[derive(Serialize)]
struct NicoHealthInventory {
    default_credentials: NicoHealthCredentials,
    nodes: Vec<NicoHealthNode>,
}

#[derive(Clone, Eq, PartialEq, Serialize)]
struct NicoHealthCredentials {
    username: String,
    password: String,
}

#[derive(Serialize)]
struct NicoHealthNode {
    #[serde(skip_serializing_if = "Option::is_none")]
    hostname: Option<String>,
    bmc_ip: std::net::IpAddr,
    bmc_mac: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    rack: Option<String>,
    uuid: String,
    #[serde(skip_serializing_if = "BTreeMap::is_empty")]
    labels: BTreeMap<String, String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    credentials: Option<NicoHealthCredentials>,
}

fn render_json(
    targets: &[Target],
    inventories: &[NodeInventory],
) -> Result<String, Box<dyn Error>> {
    let Some(first_target) = targets.first() else {
        return Err("cannot write NICo health inventory without BMC targets".into());
    };
    let default_credentials = credentials(first_target);
    let nodes = targets
        .iter()
        .map(|target| -> Result<NicoHealthNode, Box<dyn Error>> {
            let inventory = inventories
                .iter()
                .find(|inventory| inventory.source.mac.eq_ignore_ascii_case(&target.mac));
            let uuid = inventory
                .and_then(|inventory| inventory.uuid.as_ref())
                .ok_or_else(|| {
                    format!(
                        "inventory discovery did not produce a node UUID for BMC MAC {}",
                        target.mac
                    )
                })?
                .to_string();
            let target_credentials = credentials(target);
            Ok(NicoHealthNode {
                hostname: inventory.and_then(|inventory| inventory.source.host_name.clone()),
                bmc_ip: target.ip,
                bmc_mac: target.mac.clone(),
                rack: target.rack.clone(),
                uuid,
                labels: target.labels.clone(),
                credentials: (target_credentials != default_credentials)
                    .then_some(target_credentials),
            })
        })
        .collect::<Result<Vec<_>, _>>()?;

    let mut rendered = serde_json::to_string_pretty(&NicoHealthInventory {
        default_credentials,
        nodes,
    })?;
    rendered.push('\n');
    Ok(rendered)
}

fn credentials(target: &Target) -> NicoHealthCredentials {
    NicoHealthCredentials {
        username: target.username.clone(),
        password: target.password.clone(),
    }
}

fn write_atomically(path: &Path, contents: &[u8]) -> Result<(), Box<dyn Error>> {
    let parent = path.parent().unwrap_or_else(|| Path::new("."));
    let file_name = path
        .file_name()
        .and_then(|name| name.to_str())
        .ok_or("NICo health inventory output path must name a file")?;
    let nonce = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_nanos();
    let temporary_path = parent.join(format!(".{file_name}.{}.{nonce}.tmp", std::process::id()));

    let result = (|| -> Result<(), Box<dyn Error>> {
        let mut options = OpenOptions::new();
        options.write(true).create_new(true);
        #[cfg(unix)]
        options.mode(0o600);
        let mut file = options.open(&temporary_path)?;
        file.write_all(contents)?;
        file.sync_all()?;
        drop(file);
        std::fs::rename(&temporary_path, path)?;
        Ok(())
    })();

    if result.is_err() {
        let _ = std::fs::remove_file(&temporary_path);
    }
    result
}

#[cfg(test)]
mod tests {
    #[cfg(unix)]
    use std::os::unix::fs::PermissionsExt;

    use super::*;
    use crate::models::{INVENTORY_SCHEMA_VERSION, InventorySourceInfo, InventoryUuid};

    #[test]
    fn renders_nico_health_file_inventory_with_labels_and_per_node_credentials() {
        let targets = vec![
            target(
                "10.0.0.1",
                "00:11:22:33:44:55",
                "admin",
                "default-secret",
                Some("rack-01"),
                BTreeMap::from([
                    ("compute_zone".to_string(), "az51".to_string()),
                    ("node_group".to_string(), "dev3-dh1".to_string()),
                ]),
            ),
            target(
                "10.0.0.2",
                "00:11:22:33:44:66",
                "root",
                "node-secret",
                None,
                BTreeMap::new(),
            ),
        ];
        let inventories = vec![
            inventory(
                "00:11:22:33:44:55",
                Some("node-01"),
                Some("550e8400-e29b-41d4-a716-446655440000"),
            ),
            inventory(
                "00:11:22:33:44:66",
                None,
                Some("98f3a1e0-2fbb-4a89-b540-f5ee17011536"),
            ),
        ];

        let rendered = render_json(&targets, &inventories).expect("NICo health JSON");
        let value: serde_json::Value = serde_json::from_str(&rendered).expect("valid JSON");

        assert_eq!(value["default_credentials"]["username"], "admin");
        assert_eq!(value["default_credentials"]["password"], "default-secret");
        assert_eq!(value["nodes"][0]["hostname"], "node-01");
        assert_eq!(value["nodes"][0]["bmc_ip"], "10.0.0.1");
        assert_eq!(value["nodes"][0]["bmc_mac"], "00:11:22:33:44:55");
        assert_eq!(value["nodes"][0]["rack"], "rack-01");
        assert_eq!(
            value["nodes"][0]["uuid"],
            "550e8400-e29b-41d4-a716-446655440000"
        );
        assert_eq!(value["nodes"][0]["labels"]["compute_zone"], "az51");
        assert_eq!(value["nodes"][0]["labels"]["node_group"], "dev3-dh1");
        assert!(value["nodes"][0].get("credentials").is_none());
        assert_eq!(value["nodes"][1]["credentials"]["username"], "root");
        assert_eq!(value["nodes"][1]["credentials"]["password"], "node-secret");
        assert_eq!(
            value["nodes"][1]["uuid"],
            "98f3a1e0-2fbb-4a89-b540-f5ee17011536"
        );
        assert!(value["nodes"][1].get("labels").is_none());
    }

    #[test]
    fn rejects_inventory_without_a_discovered_node_uuid() {
        let targets = vec![target(
            "10.0.0.1",
            "00:11:22:33:44:55",
            "admin",
            "secret",
            None,
            BTreeMap::new(),
        )];
        let inventories = vec![inventory("00:11:22:33:44:55", None, None)];

        let error = render_json(&targets, &inventories)
            .expect_err("missing node UUID should fail")
            .to_string();

        assert!(error.contains("did not produce a node UUID"));
        assert!(error.contains("00:11:22:33:44:55"));
    }

    #[cfg(unix)]
    #[test]
    fn sink_writes_permission_restricted_inventory_file() {
        let nonce = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .expect("time after epoch")
            .as_nanos();
        let directory = std::env::temp_dir().join(format!(
            "fleetint-nico-health-sink-{}-{nonce}",
            std::process::id()
        ));
        std::fs::create_dir(&directory).expect("create test directory");
        let output = directory.join("inventory.json");
        let targets = vec![target(
            "10.0.0.1",
            "00:11:22:33:44:55",
            "admin",
            "secret",
            None,
            BTreeMap::new(),
        )];
        let inventories = vec![inventory(
            "00:11:22:33:44:55",
            None,
            Some("550e8400-e29b-41d4-a716-446655440000"),
        )];

        NicoHealthSink::new(&output, targets)
            .write(&inventories)
            .expect("write NICo health inventory");

        let permissions = std::fs::metadata(&output)
            .expect("output metadata")
            .permissions()
            .mode()
            & 0o777;
        assert_eq!(permissions, 0o600);
        let value: serde_json::Value =
            serde_json::from_slice(&std::fs::read(&output).expect("read generated inventory"))
                .expect("valid generated inventory");
        assert_eq!(value["nodes"][0]["bmc_ip"], "10.0.0.1");

        std::fs::remove_file(output).expect("remove output");
        std::fs::remove_dir(directory).expect("remove test directory");
    }

    fn target(
        ip: &str,
        mac: &str,
        username: &str,
        password: &str,
        rack: Option<&str>,
        labels: BTreeMap<String, String>,
    ) -> Target {
        Target {
            ip: ip.parse().expect("IP"),
            mac: mac.to_string(),
            address: ip.to_string(),
            username: username.to_string(),
            password: password.to_string(),
            rack: rack.map(str::to_string),
            labels,
        }
    }

    fn inventory(mac: &str, hostname: Option<&str>, uuid: Option<&str>) -> NodeInventory {
        NodeInventory {
            schema_version: INVENTORY_SCHEMA_VERSION.to_string(),
            collected_at: "2026-01-01T00:00:00Z".to_string(),
            enrolled_at: None,
            uuid: uuid.and_then(InventoryUuid::parse),
            source: InventorySourceInfo {
                source_type: "redfish".to_string(),
                mac: mac.to_string(),
                address: "10.0.0.1".to_string(),
                host_name: hostname.map(str::to_string),
                redfish_version: None,
                service_uuid: None,
                vendor: None,
            },
            primary_system_id: None,
            managers: Vec::new(),
            systems: Vec::new(),
            chassis: Vec::new(),
            firmware: Vec::new(),
            domain_errors: Vec::new(),
            target_error: None,
        }
    }
}
