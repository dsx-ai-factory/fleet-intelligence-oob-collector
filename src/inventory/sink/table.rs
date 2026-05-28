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
    vendor: String,
    manager: String,
    system: String,
    chassis: String,
    firmware: String,
    error: String,
}

#[derive(Debug)]
struct NodeDetailRow {
    target: String,
    address: String,
    vendor: String,
    manager_id: String,
    manager_ifaces: String,
    system_id: String,
    manufacturer: String,
    model: String,
    serial: String,
    power: String,
    system_ifaces: String,
    pcie_devices: String,
}

#[derive(Debug)]
struct ChassisDetailRow {
    id: String,
    manufacturer: String,
    model: String,
    serial: String,
    adapters: String,
}

#[derive(Debug)]
struct FirmwareDetailRow {
    service: String,
    id: String,
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
                "Target", "Address", "Vendor", "Manager", "System", "Chassis", "Firmware", "Error",
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

        Self {
            target: inventory.target.clone(),
            address: inventory.address.clone(),
            vendor: inventory.vendor.clone().unwrap_or_default(),
            manager,
            system,
            chassis: inventory.chassis.len().to_string(),
            firmware: inventory.firmware.len().to_string(),
            error: inventory.error.clone().unwrap_or_else(|| "0".to_string()),
        }
    }
}

impl SummaryRow {
    fn values(&self) -> [String; 8] {
        [
            self.target.clone(),
            self.address.clone(),
            self.vendor.clone(),
            self.manager.clone(),
            self.system.clone(),
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
            manager_id: inventory
                .manager
                .as_ref()
                .map(|manager| manager.id.clone())
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
            power: inventory
                .system
                .as_ref()
                .map(|system| system.power_state.clone())
                .unwrap_or_default(),
            system_ifaces: inventory
                .system
                .as_ref()
                .map(|system| system.interface_count.to_string())
                .unwrap_or_default(),
            pcie_devices: inventory
                .system
                .as_ref()
                .map(|system| system.pcie_device_count.to_string())
                .unwrap_or_default(),
        }
    }
}

impl NodeDetailRow {
    fn values(&self) -> [String; 12] {
        [
            self.target.clone(),
            self.address.clone(),
            self.vendor.clone(),
            self.manager_id.clone(),
            self.manager_ifaces.clone(),
            self.system_id.clone(),
            self.manufacturer.clone(),
            self.model.clone(),
            self.serial.clone(),
            self.power.clone(),
            self.system_ifaces.clone(),
            self.pcie_devices.clone(),
        ]
    }
}

impl From<&ChassisInventory> for ChassisDetailRow {
    fn from(chassis: &ChassisInventory) -> Self {
        Self {
            id: chassis.id.clone(),
            manufacturer: chassis.manufacturer.clone().unwrap_or_default(),
            model: chassis.model.clone().unwrap_or_default(),
            serial: chassis.serial_number.clone().unwrap_or_default(),
            adapters: chassis.network_adapter_count.to_string(),
        }
    }
}

impl ChassisDetailRow {
    fn values(&self) -> [String; 5] {
        [
            self.id.clone(),
            self.manufacturer.clone(),
            self.model.clone(),
            self.serial.clone(),
            self.adapters.clone(),
        ]
    }
}

impl From<&FirmwareInventory> for FirmwareDetailRow {
    fn from(firmware: &FirmwareInventory) -> Self {
        Self {
            service: firmware.service_id.clone(),
            id: firmware.id.clone(),
            version: firmware.version.clone().unwrap_or_default(),
            release_date: firmware.release_date.clone().unwrap_or_default(),
        }
    }
}

impl FirmwareDetailRow {
    fn values(&self) -> [String; 4] {
        [
            self.service.clone(),
            self.id.clone(),
            self.version.clone(),
            self.release_date.clone(),
        ]
    }
}

fn print_node_details(inventory: &NodeInventory) {
    println!();
    println!("Node: {} ({})", inventory.target, inventory.address);

    let node_row = NodeDetailRow::from(inventory);
    print_table(
        &[
            "Target",
            "Address",
            "Vendor",
            "Manager",
            "Mgr Ifaces",
            "System",
            "Manufacturer",
            "Model",
            "Serial",
            "Power",
            "Sys Ifaces",
            "PCIe",
        ],
        std::iter::once(node_row.values()),
    );

    let chassis_rows = inventory
        .chassis
        .iter()
        .map(ChassisDetailRow::from)
        .map(|row| row.values());
    println!("Chassis");
    print_table(
        &["ID", "Manufacturer", "Model", "Serial", "Adapters"],
        chassis_rows,
    );

    let firmware_rows = inventory
        .firmware
        .iter()
        .map(FirmwareDetailRow::from)
        .map(|row| row.values());
    println!("Firmware");
    print_table(&["Service", "ID", "Version", "Release Date"], firmware_rows);
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
