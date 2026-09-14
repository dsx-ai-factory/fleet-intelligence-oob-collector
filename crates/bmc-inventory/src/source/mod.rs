mod redfish;

use async_trait::async_trait;

use crate::config::Target;

pub use redfish::RedfishSource;

pub mod inventory_domain {
    pub const MANAGERS: &str = "managers";
    pub const SYSTEMS: &str = "systems";
    pub const SYSTEMS_PROCESSORS: &str = "systems.processors";
    pub const SYSTEMS_SECURE_BOOT: &str = "systems.secureBoot";
    pub const CHASSIS: &str = "chassis";
    pub const CHASSIS_PCIE_DEVICES: &str = "chassis.pcieDevices";
    pub const FIRMWARE: &str = "firmware";
}

pub struct SourceRecord {
    pub target: Target,
    pub collected_at: String,
    pub result: Result<RedfishReport, String>,
}

#[async_trait]
pub trait InventorySource {
    async fn collect(&self, target: Target) -> SourceRecord;
}

#[derive(Clone, Debug)]
pub struct RedfishReport {
    pub service_root: ServiceRootData,
    pub managers: Vec<ManagerData>,
    pub systems: Vec<SystemData>,
    pub chassis: Vec<ChassisData>,
    pub firmware: Vec<FirmwareData>,
    pub errors: Vec<InventoryErrorData>,
}

#[derive(Clone, Debug, Default)]
pub struct ServiceRootData {
    pub redfish_version: Option<String>,
    pub uuid: Option<String>,
    pub vendor: Option<String>,
    pub product: Option<String>,
}

#[derive(Clone, Debug)]
pub struct ManagerData {
    pub id: String,
    pub odata_id: Option<String>,
    pub uuid: Option<String>,
    pub manager_type: Option<String>,
    pub model: Option<String>,
    pub firmware_version: Option<String>,
    pub status: Option<StatusData>,
}

#[derive(Clone, Debug)]
pub struct SystemData {
    pub id: String,
    pub odata_id: Option<String>,
    pub uuid: Option<String>,
    pub manufacturer: Option<String>,
    pub model: Option<String>,
    pub sku: Option<String>,
    pub serial_number: Option<String>,
    pub bios_version: Option<String>,
    pub host_name: Option<String>,
    pub asset_tag: Option<String>,
    pub power_state: Option<String>,
    pub status: Option<StatusData>,
    pub cpu_count: Option<i64>,
    pub cpu_core_count: Option<i64>,
    pub cpu_model: Option<String>,
    pub memory_gib: Option<f64>,
    pub secure_boot_enabled: Option<bool>,
    pub processors: Vec<ProcessorData>,
}

#[derive(Clone, Debug)]
pub struct ProcessorData {
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
    pub status: Option<StatusData>,
}

#[derive(Clone, Debug)]
pub struct ChassisData {
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
    pub status: Option<StatusData>,
    pub location: Option<ChassisLocationData>,
    pub pcie_devices: Vec<PcieDeviceData>,
}

#[derive(Clone, Debug, Default)]
pub struct ChassisLocationData {
    pub service_label: Option<String>,
    pub rack: Option<String>,
    pub row: Option<String>,
    pub room: Option<String>,
    pub rack_offset: Option<i64>,
}

#[derive(Clone, Debug)]
pub struct PcieDeviceData {
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
    pub status: Option<StatusData>,
}

#[derive(Clone, Debug)]
pub struct FirmwareData {
    pub service_id: String,
    pub id: String,
    pub odata_id: Option<String>,
    pub name: String,
    pub version: Option<String>,
    pub release_date: Option<String>,
    pub status: Option<StatusData>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct StatusData {
    pub conditions: Option<Vec<ConditionData>>,
    pub health: Option<String>,
    pub health_rollup: Option<String>,
    pub state: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ConditionData {
    pub timestamp: Option<String>,
    pub message_id: String,
    pub message_args: Option<Vec<String>>,
    pub message: Option<String>,
    pub severity: Option<String>,
    pub resolution: Option<String>,
    pub condition_type: Option<String>,
    pub origin_of_condition: Option<String>,
}

#[derive(Clone, Debug)]
pub struct InventoryErrorData {
    pub domain: String,
    pub resource: Option<String>,
    pub message: String,
}
