use std::fmt;

use uuid::Uuid;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SystemUuid(Uuid);

impl SystemUuid {
    pub fn parse(value: &str) -> Option<Self> {
        Uuid::parse_str(value.trim()).ok().map(Self)
    }
}

impl fmt::Display for SystemUuid {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.0.fmt(f)
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ChassisSerial(String);

impl ChassisSerial {
    pub fn parse(value: &str) -> Option<Self> {
        let value = value.trim();
        if value.is_empty() {
            None
        } else {
            Some(Self(value.to_string()))
        }
    }
}

impl fmt::Display for ChassisSerial {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

#[derive(Debug)]
pub struct NodeInventory {
    pub target: String,
    pub address: String,
    pub redfish_version: Option<String>,
    pub service_uuid: Option<String>,
    pub vendor: Option<String>,
    pub manager: Option<ManagerInventory>,
    pub system: Option<SystemInventory>,
    pub chassis: Vec<ChassisInventory>,
    pub firmware: Vec<FirmwareInventory>,
    pub error: Option<String>,
}

#[derive(Debug)]
pub struct ManagerInventory {
    pub id: String,
    pub uuid: Option<String>,
    pub manager_type: Option<String>,
    pub model: Option<String>,
    pub firmware_version: Option<String>,
    pub interface_count: usize,
}

#[derive(Debug)]
pub struct SystemInventory {
    pub id: String,
    pub uuid: Option<SystemUuid>,
    pub manufacturer: Option<String>,
    pub model: Option<String>,
    pub serial_number: Option<String>,
    pub bios_version: Option<String>,
    pub power_state: String,
    pub cpu_count: Option<i64>,
    pub cpu_core_count: Option<i64>,
    pub cpu_model: Option<String>,
    pub memory_gib: Option<f64>,
    pub interface_count: usize,
    pub pcie_device_count: usize,
}

#[derive(Debug)]
pub struct ChassisInventory {
    pub id: String,
    pub chassis_type: Option<String>,
    pub manufacturer: Option<String>,
    pub model: Option<String>,
    pub serial_number: Option<ChassisSerial>,
    pub part_number: Option<String>,
    pub network_adapter_count: usize,
}

#[derive(Debug)]
pub struct FirmwareInventory {
    pub service_id: String,
    pub id: String,
    pub name: String,
    pub version: Option<String>,
    pub release_date: Option<String>,
}
