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
    pub name: String,
    pub description: Option<String>,
    pub id: String,
    #[serde(rename = "@odata.id")]
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
    pub name: String,
    pub description: Option<String>,
    pub id: String,
    #[serde(rename = "@odata.id")]
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
    pub processor_summary: Option<ProcessorSummary>,
    pub memory_summary: Option<MemorySummary>,
    pub secure_boot_enabled: Option<bool>,
    pub processors: Vec<ProcessorInventory>,
    pub links: std::collections::BTreeMap<String, InventoryLink>,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ProcessorInventory {
    pub name: String,
    pub description: Option<String>,
    pub id: String,
    #[serde(rename = "@odata.id")]
    pub odata_id: Option<String>,
    pub uuid: Option<String>,
    pub socket: Option<String>,
    pub processor_type: Option<String>,
    pub processor_architecture: Option<String>,
    pub instruction_set: Option<String>,
    pub manufacturer: Option<String>,
    pub model: Option<String>,
    pub serial_number: Option<String>,
    pub part_number: Option<String>,
    pub location: Option<ChassisLocation>,
    pub nvidia_gpu: Option<NvidiaGpuInventory>,
    pub max_speed_mhz: Option<i64>,
    pub total_cores: Option<i64>,
    pub total_threads: Option<i64>,
    pub status: Option<InventoryStatus>,
    pub links: std::collections::BTreeMap<String, InventoryLink>,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct NvidiaGpuInventory {
    pub mig_mode_enabled: Option<bool>,
    pub default_boost_clock_speed_mhz: Option<i64>,
    pub max_boost_clock_speed_mhz: Option<i64>,
    pub cc_mode_enabled: Option<bool>,
    pub cc_dev_mode_enabled: Option<bool>,
    pub protected_pcie_mode_enabled: Option<bool>,
    pub egm_mode_enabled: Option<bool>,
    pub total_nv_links: Option<i64>,
    pub nvlink_topology: Option<NvlinkTopologyInventory>,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct NvlinkTopologyInventory {
    pub tray_serial_number: Option<String>,
    pub tray_slot_number: Option<i64>,
    pub tray_slot_index: Option<i64>,
    pub chassis_serial_number: Option<String>,
    pub host_id: Option<i64>,
    pub module_id: Option<i64>,
    pub system_guid: Option<String>,
    pub ib_guid: Option<String>,
    pub lsb_ib_guid: Option<String>,
    pub peer_type: Option<String>,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ChassisInventory {
    pub name: String,
    pub description: Option<String>,
    pub id: String,
    #[serde(rename = "@odata.id")]
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
    pub part_location: Option<PartLocation>,
    pub placement: Option<Placement>,
    pub part_location_context: Option<String>,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PcieDeviceInventory {
    pub name: String,
    pub description: Option<String>,
    pub id: String,
    #[serde(rename = "@odata.id")]
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
    pub description: Option<String>,
    pub related_item: Option<Vec<InventoryLinkReference>>,
    pub service_id: String,
    pub id: String,
    #[serde(rename = "@odata.id")]
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

#[derive(Debug, Default, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ProcessorSummary {
    pub count: Option<i64>,
    pub core_count: Option<i64>,
    pub model: Option<String>,
    pub logical_processor_count: Option<i64>,
    pub threading_enabled: Option<bool>,
    pub status: Option<InventoryStatus>,
}

#[derive(Debug, Default, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct MemorySummary {
    #[serde(rename = "totalSystemMemoryGiB")]
    pub total_system_memory_gib: Option<f64>,
    #[serde(rename = "totalSystemPersistentMemoryGiB")]
    pub total_system_persistent_memory_gib: Option<f64>,
    pub memory_mirroring: Option<String>,
    pub status: Option<InventoryStatus>,
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

#[derive(Debug, Default, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PartLocation {
    pub service_label: Option<String>,
    pub location_type: Option<String>,
    pub location_ordinal_value: Option<i64>,
    pub reference: Option<String>,
    pub orientation: Option<String>,
}

#[derive(Debug, Default, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Placement {
    pub rack: Option<String>,
    pub row: Option<String>,
    pub room: Option<String>,
    pub rack_offset: Option<i64>,
    pub rack_offset_units: Option<String>,
    pub additional_info: Option<String>,
    pub facility_name: Option<String>,
}
