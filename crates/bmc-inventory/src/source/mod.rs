mod assembly;
mod redfish;
mod reverse_dns;

use async_trait::async_trait;

use crate::config::Target;
use crate::models::{
    ChassisInventory, DomainError, FabricInventory, FirmwareInventory, ManagerInventory,
    NodeInventory, SystemInventory,
};

pub use redfish::RedfishSource;

pub mod inventory_domain {
    pub const MANAGERS: &str = "managers";
    pub const SYSTEMS: &str = "systems";
    pub const SYSTEMS_PROCESSORS: &str = "systems.processors";
    pub const SYSTEMS_SECURE_BOOT: &str = "systems.secureBoot";
    pub const CHASSIS: &str = "chassis";
    pub const CHASSIS_PCIE_DEVICES: &str = "chassis.pcieDevices";
    pub const FIRMWARE: &str = "firmware";
    pub const FABRICS: &str = "fabrics";
    pub const FABRICS_SWITCHES: &str = "fabrics.switches";
}

#[async_trait]
pub trait InventorySource {
    async fn collect(&self, target: Target) -> NodeInventory;
}

/// Groups canonical resources with service metadata and recoverable failures
/// until node identity can be assigned.
struct RedfishReport {
    service_root: ServiceRootData,
    managers: Vec<ManagerInventory>,
    systems: Vec<SystemInventory>,
    chassis: Vec<ChassisInventory>,
    firmware: Vec<FirmwareInventory>,
    fabrics: Vec<FabricInventory>,
    errors: Vec<DomainError>,
}

struct ServiceRootData {
    redfish_version: Option<String>,
    uuid: Option<String>,
    vendor: Option<String>,
    product: Option<String>,
}
