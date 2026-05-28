#[derive(Debug)]
pub struct NodeInventory {
    pub target: String,
    pub address: String,
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
    pub interface_count: usize,
}

#[derive(Debug)]
pub struct SystemInventory {
    pub id: String,
    pub manufacturer: Option<String>,
    pub model: Option<String>,
    pub serial_number: Option<String>,
    pub power_state: String,
    pub interface_count: usize,
    pub pcie_device_count: usize,
}

#[derive(Debug)]
pub struct ChassisInventory {
    pub id: String,
    pub manufacturer: Option<String>,
    pub model: Option<String>,
    pub serial_number: Option<String>,
    pub network_adapter_count: usize,
}

#[derive(Debug)]
pub struct FirmwareInventory {
    pub service_id: String,
    pub id: String,
    pub version: Option<String>,
    pub release_date: Option<String>,
}
