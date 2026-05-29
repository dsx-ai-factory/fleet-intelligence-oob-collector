use std::error::Error;
use std::fmt;

use super::InventorySink;
use crate::inventory::types::{ChassisInventory, FirmwareInventory, NodeInventory};

#[derive(Default)]
pub struct TableSink {
    detail: bool,
}

#[derive(Debug)]
struct SummaryRow {
    target: String,
    address: String,
    redfish: String,
    service_uuid: String,
    vendor: String,
    manager: String,
    system: String,
    system_uuid: String,
    chassis: String,
    firmware: String,
    error: String,
}

#[derive(Debug)]
struct NodeDetailRow {
    target: String,
    address: String,
    vendor: String,
    redfish: String,
    service_uuid: String,
    manager_id: String,
    manager_uuid: String,
    manager_type: String,
    manager_model: String,
    manager_firmware: String,
    manager_ifaces: String,
    system_id: String,
    system_uuid: String,
    manufacturer: String,
    model: String,
    serial: String,
    bios: String,
    power: String,
    cpu: String,
    memory_gib: String,
    system_ifaces: String,
}

#[derive(Debug)]
struct ChassisDetailRow {
    id: String,
    chassis_type: String,
    manufacturer: String,
    model: String,
    serial: String,
    part: String,
    adapters: String,
}

#[derive(Debug)]
struct FirmwareDetailRow {
    service: String,
    id: String,
    name: String,
    version: String,
    release_date: String,
}

impl TableSink {
    pub fn new(detail: bool) -> Self {
        Self { detail }
    }
}

impl InventorySink for TableSink {
    fn write(&self, inventories: &[NodeInventory]) -> Result<(), Box<dyn Error>> {
        let rows = inventories.iter().map(SummaryRow::from).collect::<Vec<_>>();
        print_table(
            &[
                "Target",
                "Address",
                "Redfish",
                "Service UUID",
                "Vendor",
                "Manager",
                "System",
                "System UUID",
                "Chassis",
                "Firmware",
                "Error",
            ],
            rows.iter().map(SummaryRow::values),
        );

        if self.detail {
            for inventory in inventories {
                print_node_details(inventory);
            }
        }

        Ok(())
    }
}

impl From<&NodeInventory> for SummaryRow {
    fn from(inventory: &NodeInventory) -> Self {
        let manager = inventory
            .manager
            .as_ref()
            .map(|manager| manager.id.clone())
            .unwrap_or_default();
        let system = inventory
            .system
            .as_ref()
            .map(|system| system.id.clone())
            .unwrap_or_default();
        let system_uuid = inventory
            .system
            .as_ref()
            .and_then(|system| system.uuid.clone())
            .unwrap_or_default();

        Self {
            target: inventory.target.clone(),
            address: inventory.address.clone(),
            redfish: inventory.redfish_version.clone().unwrap_or_default(),
            service_uuid: inventory.service_uuid.clone().unwrap_or_default(),
            vendor: inventory.vendor.clone().unwrap_or_default(),
            manager,
            system,
            system_uuid,
            chassis: inventory.chassis.len().to_string(),
            firmware: inventory.firmware.len().to_string(),
            error: inventory.error.clone().unwrap_or_else(|| "0".to_string()),
        }
    }
}

impl SummaryRow {
    fn values(&self) -> [String; 11] {
        [
            self.target.clone(),
            self.address.clone(),
            self.redfish.clone(),
            self.service_uuid.clone(),
            self.vendor.clone(),
            self.manager.clone(),
            self.system.clone(),
            self.system_uuid.clone(),
            self.chassis.clone(),
            self.firmware.clone(),
            self.error.clone(),
        ]
    }
}

impl From<&NodeInventory> for NodeDetailRow {
    fn from(inventory: &NodeInventory) -> Self {
        Self {
            target: inventory.target.clone(),
            address: inventory.address.clone(),
            vendor: inventory.vendor.clone().unwrap_or_default(),
            redfish: inventory.redfish_version.clone().unwrap_or_default(),
            service_uuid: inventory.service_uuid.clone().unwrap_or_default(),
            manager_id: inventory
                .manager
                .as_ref()
                .map(|manager| manager.id.clone())
                .unwrap_or_default(),
            manager_uuid: inventory
                .manager
                .as_ref()
                .and_then(|manager| manager.uuid.clone())
                .unwrap_or_default(),
            manager_type: inventory
                .manager
                .as_ref()
                .and_then(|manager| manager.manager_type.clone())
                .unwrap_or_default(),
            manager_model: inventory
                .manager
                .as_ref()
                .and_then(|manager| manager.model.clone())
                .unwrap_or_default(),
            manager_firmware: inventory
                .manager
                .as_ref()
                .and_then(|manager| manager.firmware_version.clone())
                .unwrap_or_default(),
            manager_ifaces: inventory
                .manager
                .as_ref()
                .map(|manager| manager.interface_count.to_string())
                .unwrap_or_default(),
            system_id: inventory
                .system
                .as_ref()
                .map(|system| system.id.clone())
                .unwrap_or_default(),
            system_uuid: inventory
                .system
                .as_ref()
                .and_then(|system| system.uuid.clone())
                .unwrap_or_default(),
            manufacturer: inventory
                .system
                .as_ref()
                .and_then(|system| system.manufacturer.clone())
                .unwrap_or_default(),
            model: inventory
                .system
                .as_ref()
                .and_then(|system| system.model.clone())
                .unwrap_or_default(),
            serial: inventory
                .system
                .as_ref()
                .and_then(|system| system.serial_number.clone())
                .unwrap_or_default(),
            bios: inventory
                .system
                .as_ref()
                .and_then(|system| system.bios_version.clone())
                .unwrap_or_default(),
            power: inventory
                .system
                .as_ref()
                .map(|system| system.power_state.clone())
                .unwrap_or_default(),
            cpu: inventory
                .system
                .as_ref()
                .map(format_cpu)
                .unwrap_or_default(),
            memory_gib: inventory
                .system
                .as_ref()
                .and_then(|system| system.memory_gib)
                .map(|memory| memory.to_string())
                .unwrap_or_default(),
            system_ifaces: inventory
                .system
                .as_ref()
                .map(|system| system.interface_count.to_string())
                .unwrap_or_default(),
        }
    }
}

impl NodeDetailRow {
    fn values(&self) -> Vec<[String; 2]> {
        vec![
            ["Target".to_string(), self.target.clone()],
            ["Address".to_string(), self.address.clone()],
            ["Vendor".to_string(), self.vendor.clone()],
            ["Redfish".to_string(), self.redfish.clone()],
            ["Service UUID".to_string(), self.service_uuid.clone()],
            ["Manager".to_string(), self.manager_id.clone()],
            ["Manager UUID".to_string(), self.manager_uuid.clone()],
            ["Manager Type".to_string(), self.manager_type.clone()],
            ["Manager Model".to_string(), self.manager_model.clone()],
            ["BMC FW".to_string(), self.manager_firmware.clone()],
            ["Mgr Ifaces".to_string(), self.manager_ifaces.clone()],
            ["System".to_string(), self.system_id.clone()],
            ["System UUID".to_string(), self.system_uuid.clone()],
            ["Manufacturer".to_string(), self.manufacturer.clone()],
            ["Model".to_string(), self.model.clone()],
            ["Serial".to_string(), self.serial.clone()],
            ["BIOS".to_string(), self.bios.clone()],
            ["Power".to_string(), self.power.clone()],
            ["CPU".to_string(), self.cpu.clone()],
            ["Mem GiB".to_string(), self.memory_gib.clone()],
            ["Sys Ifaces".to_string(), self.system_ifaces.clone()],
        ]
    }
}

impl From<&ChassisInventory> for ChassisDetailRow {
    fn from(chassis: &ChassisInventory) -> Self {
        Self {
            id: chassis.id.clone(),
            chassis_type: chassis.chassis_type.clone().unwrap_or_default(),
            manufacturer: chassis.manufacturer.clone().unwrap_or_default(),
            model: chassis.model.clone().unwrap_or_default(),
            serial: chassis.serial_number.clone().unwrap_or_default(),
            part: chassis.part_number.clone().unwrap_or_default(),
            adapters: chassis.network_adapter_count.to_string(),
        }
    }
}

impl ChassisDetailRow {
    fn values(&self) -> [String; 7] {
        [
            self.id.clone(),
            self.chassis_type.clone(),
            self.manufacturer.clone(),
            self.model.clone(),
            self.serial.clone(),
            self.part.clone(),
            self.adapters.clone(),
        ]
    }
}

impl From<&FirmwareInventory> for FirmwareDetailRow {
    fn from(firmware: &FirmwareInventory) -> Self {
        Self {
            service: firmware.service_id.clone(),
            id: firmware.id.clone(),
            name: firmware.name.clone(),
            version: firmware.version.clone().unwrap_or_default(),
            release_date: firmware.release_date.clone().unwrap_or_default(),
        }
    }
}

impl FirmwareDetailRow {
    fn values(&self) -> [String; 5] {
        [
            self.service.clone(),
            self.id.clone(),
            self.name.clone(),
            self.version.clone(),
            self.release_date.clone(),
        ]
    }
}

fn print_node_details(inventory: &NodeInventory) {
    println!();
    println!("Node: {} ({})", inventory.target, inventory.address);

    let node_row = NodeDetailRow::from(inventory);
    print_table(&["Attribute", "Value"], node_row.values());

    let chassis_rows = inventory
        .chassis
        .iter()
        .map(ChassisDetailRow::from)
        .map(|row| row.values());
    println!("Chassis");
    print_table(
        &[
            "ID",
            "Type",
            "Manufacturer",
            "Model",
            "Serial",
            "Part",
            "Adapters",
        ],
        chassis_rows,
    );

    let firmware_rows = inventory
        .firmware
        .iter()
        .map(FirmwareDetailRow::from)
        .map(|row| row.values());
    println!("Firmware");
    print_table(
        &["Service", "ID", "Name", "Version", "Release Date"],
        firmware_rows,
    );
}

fn format_cpu(system: &crate::inventory::types::SystemInventory) -> String {
    match (system.cpu_count, system.cpu_core_count) {
        (Some(count), Some(cores)) => format!("{count}/{cores}"),
        (Some(count), None) => count.to_string(),
        (None, Some(cores)) => format!("?/{cores}"),
        (None, None) => String::new(),
    }
}

fn print_table<const N: usize>(headers: &[&str; N], rows: impl IntoIterator<Item = [String; N]>) {
    let rows = rows.into_iter().collect::<Vec<_>>();
    let mut widths = headers.map(str::len);
    for row in &rows {
        for (idx, value) in row.iter().enumerate() {
            widths[idx] = widths[idx].max(value.len());
        }
    }

    print_separator(&widths);
    print_row(headers.as_slice(), &widths);
    print_separator(&widths);
    for row in &rows {
        print_row(row, &widths);
    }
    print_separator(&widths);
}

fn print_separator(widths: &[usize]) {
    print!("+");
    for width in widths {
        print!("{}+", "-".repeat(width + 2));
    }
    println!();
}

fn print_row<T: fmt::Display>(values: &[T], widths: &[usize]) {
    print!("|");
    for (value, width) in values.iter().zip(widths) {
        print!(" {value:<width$} |");
    }
    println!();
}
