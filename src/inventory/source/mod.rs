mod redfish;

use async_trait::async_trait;

use crate::config::Target;

pub use redfish::RedfishSource;

pub struct SourceRecord {
    pub target: Target,
    pub result: Result<RedfishReport, String>,
}

#[async_trait]
pub trait InventorySource {
    async fn collect(&self, target: Target) -> SourceRecord;
}

#[derive(Debug)]
pub struct RedfishReport {
    pub service_root: ServiceRootData,
    pub managers: Vec<ManagerData>,
    pub systems: Vec<SystemData>,
    pub chassis: Vec<ChassisData>,
    pub firmware: Vec<FirmwareData>,
}

#[derive(Debug, Default)]
pub struct ServiceRootData {
    pub redfish_version: Option<String>,
    pub uuid: Option<String>,
    pub vendor: Option<String>,
    pub product: Option<String>,
}

#[derive(Debug)]
pub struct ManagerData {
    pub id: String,
    pub uuid: Option<String>,
    pub manager_type: Option<String>,
    pub model: Option<String>,
    pub firmware_version: Option<String>,
    pub interface_count: usize,
}

#[derive(Debug)]
pub struct SystemData {
    pub id: String,
    pub uuid: Option<String>,
    pub manufacturer: Option<String>,
    pub model: Option<String>,
    pub serial_number: Option<String>,
    pub bios_version: Option<String>,
    pub power_state: Option<String>,
    pub cpu_count: Option<i64>,
    pub cpu_core_count: Option<i64>,
    pub cpu_model: Option<String>,
    pub memory_gib: Option<f64>,
    pub interface_count: usize,
    pub pcie_device_count: usize,
}

#[derive(Debug)]
pub struct ChassisData {
    pub id: String,
    pub chassis_type: Option<String>,
    pub manufacturer: Option<String>,
    pub model: Option<String>,
    pub serial_number: Option<String>,
    pub part_number: Option<String>,
    pub network_adapter_count: usize,
}

#[derive(Debug)]
pub struct FirmwareData {
    pub service_id: String,
    pub id: String,
    pub name: String,
    pub version: Option<String>,
    pub release_date: Option<String>,
}
