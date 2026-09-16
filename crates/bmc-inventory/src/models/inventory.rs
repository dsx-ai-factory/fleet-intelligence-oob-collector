use serde::Serialize;

use super::InventoryUuid;

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct InventorySourceInfo {
    pub source_type: String,
    pub mac: String,
    pub address: String,
    pub host_name: Option<String>,
    pub redfish_version: Option<String>,
    pub service_uuid: Option<String>,
    pub vendor: Option<String>,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ManagerInventory {
    pub id: String,
    pub odata_id: Option<String>,
    pub uuid: Option<String>,
    pub manager_type: Option<String>,
    pub model: Option<String>,
    pub firmware_version: Option<String>,
    pub status: Option<InventoryStatus>,
    pub links: std::collections::BTreeMap<String, InventoryLink>,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SystemInventory {
    pub id: String,
    pub odata_id: Option<String>,
    pub uuid: Option<InventoryUuid>,
    pub manufacturer: Option<String>,
    pub model: Option<String>,
    pub sku: Option<String>,
    pub serial_number: Option<String>,
    pub bios_version: Option<String>,
    pub host_name: Option<String>,
    pub asset_tag: Option<String>,
    pub power_state: Option<String>,
    pub status: Option<InventoryStatus>,
    pub cpu_count: Option<i64>,
    pub cpu_core_count: Option<i64>,
    pub cpu_model: Option<String>,
    pub memory_gib: Option<f64>,
    pub secure_boot_enabled: Option<bool>,
    pub processors: Vec<ProcessorInventory>,
    pub links: std::collections::BTreeMap<String, InventoryLink>,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ProcessorInventory {
    pub id: String,
    pub odata_id: Option<String>,
    pub uuid: Option<String>,
    pub socket: Option<String>,
    pub processor_type: Option<String>,
    pub processor_architecture: Option<String>,
    pub instruction_set: Option<String>,
    pub manufacturer: Option<String>,
    pub model: Option<String>,
    pub serial_number: Option<String>,
    pub max_speed_mhz: Option<i64>,
    pub total_cores: Option<i64>,
    pub total_threads: Option<i64>,
    pub status: Option<InventoryStatus>,
    pub links: std::collections::BTreeMap<String, InventoryLink>,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ChassisInventory {
    pub id: String,
    pub odata_id: Option<String>,
    pub chassis_type: Option<String>,
    pub manufacturer: Option<String>,
    pub model: Option<String>,
    pub sku: Option<String>,
    pub serial_number: Option<String>,
    pub part_number: Option<String>,
    pub asset_tag: Option<String>,
    pub power_state: Option<String>,
    pub status: Option<InventoryStatus>,
    pub location: Option<ChassisLocation>,
    pub pcie_devices: Vec<PcieDeviceInventory>,
    pub links: std::collections::BTreeMap<String, InventoryLink>,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ChassisLocation {
    pub service_label: Option<String>,
    pub rack: Option<String>,
    pub row: Option<String>,
    pub room: Option<String>,
    pub rack_offset: Option<i64>,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PcieDeviceInventory {
    pub id: String,
    pub odata_id: Option<String>,
    pub uuid: Option<String>,
    pub device_type: Option<String>,
    pub manufacturer: Option<String>,
    pub model: Option<String>,
    pub sku: Option<String>,
    pub serial_number: Option<String>,
    pub part_number: Option<String>,
    pub firmware_version: Option<String>,
    pub status: Option<InventoryStatus>,
    pub links: std::collections::BTreeMap<String, InventoryLink>,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct FirmwareInventory {
    pub related_item: Option<Vec<InventoryLinkReference>>,
    pub service_id: String,
    pub id: String,
    pub odata_id: Option<String>,
    pub name: String,
    pub version: Option<String>,
    pub release_date: Option<String>,
    pub status: Option<InventoryStatus>,
    pub links: std::collections::BTreeMap<String, InventoryLink>,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct InventoryStatus {
    pub conditions: Option<Vec<InventoryCondition>>,
    pub health: Option<String>,
    pub health_rollup: Option<String>,
    pub state: Option<String>,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct InventoryCondition {
    pub timestamp: Option<String>,
    pub message_id: String,
    pub message_args: Option<Vec<String>>,
    pub message: Option<String>,
    pub severity: Option<String>,
    pub resolution: Option<String>,
    pub condition_type: Option<String>,
    pub origin_of_condition: Option<InventoryReference>,
}

#[derive(Debug, Serialize)]
pub struct InventoryReference {
    #[serde(rename = "@odata.id")]
    pub odata_id: String,
}

/// A relationship retains the single-reference or array shape supplied by Redfish.
#[derive(Debug, Serialize)]
#[serde(untagged)]
pub enum InventoryLink {
    Single(InventoryLinkReference),
    Multiple(Vec<InventoryLinkReference>),
}

#[derive(Debug, Serialize)]
pub struct InventoryLinkReference {
    #[serde(rename = "@odata.id")]
    pub odata_id: String,
}
