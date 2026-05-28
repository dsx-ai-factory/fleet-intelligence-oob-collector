use std::error::Error;
use std::fmt;

use super::InventorySink;
use crate::inventory::types::NodeInventory;

#[derive(Default)]
pub struct TableSink;

#[derive(Debug)]
struct Row {
    target: String,
    address: String,
    vendor: String,
    manager: String,
    system: String,
    chassis: String,
    firmware: String,
    error: String,
}

impl InventorySink for TableSink {
    fn write(&self, inventories: &[NodeInventory]) -> Result<(), Box<dyn Error>> {
        let rows = inventories.iter().map(Row::from).collect::<Vec<_>>();
        print_table(&rows);
        Ok(())
    }
}

impl From<&NodeInventory> for Row {
    fn from(inventory: &NodeInventory) -> Self {
        let manager = inventory
            .manager
            .as_ref()
            .map(|manager| format!("{} ({})", manager.id, manager.interface_count))
            .unwrap_or_default();
        let system = inventory
            .system
            .as_ref()
            .map(|system| {
                format!(
                    "{} {} {} {} state={} if={} pcie={}",
                    system.id,
                    system.manufacturer.as_deref().unwrap_or(""),
                    system.model.as_deref().unwrap_or(""),
                    system.serial_number.as_deref().unwrap_or(""),
                    system.power_state,
                    system.interface_count,
                    system.pcie_device_count
                )
                .split_whitespace()
                .collect::<Vec<_>>()
                .join(" ")
            })
            .unwrap_or_default();

        Self {
            target: inventory.target.clone(),
            address: inventory.address.clone(),
            vendor: inventory.vendor.clone().unwrap_or_default(),
            manager,
            system,
            chassis: summarize_chassis(inventory),
            firmware: summarize_firmware(inventory),
            error: inventory.error.clone().unwrap_or_else(|| "0".to_string()),
        }
    }
}

fn summarize_chassis(inventory: &NodeInventory) -> String {
    let Some(chassis) = inventory.chassis.first() else {
        return "0".to_string();
    };
    format!(
        "{} {} {} {} {} adapters={}",
        inventory.chassis.len(),
        chassis.id,
        chassis.manufacturer.as_deref().unwrap_or(""),
        chassis.model.as_deref().unwrap_or(""),
        chassis.serial_number.as_deref().unwrap_or(""),
        chassis.network_adapter_count
    )
    .split_whitespace()
    .collect::<Vec<_>>()
    .join(" ")
}

fn summarize_firmware(inventory: &NodeInventory) -> String {
    let Some(firmware) = inventory.firmware.first() else {
        return "0".to_string();
    };
    format!(
        "{} {} {} {} {}",
        inventory.firmware.len(),
        firmware.service_id,
        firmware.id,
        firmware.version.as_deref().unwrap_or(""),
        firmware.release_date.as_deref().unwrap_or("")
    )
    .split_whitespace()
    .collect::<Vec<_>>()
    .join(" ")
}

impl Row {
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

fn print_table(rows: &[Row]) {
    let headers = [
        "Target", "Address", "Vendor", "Manager", "System", "Chassis", "Firmware", "Error",
    ];
    let mut widths = headers.map(str::len);
    for row in rows {
        let values = row.values();
        for (idx, value) in values.iter().enumerate() {
            widths[idx] = widths[idx].max(value.len());
        }
    }

    print_separator(&widths);
    print_row(&headers, &widths);
    print_separator(&widths);
    for row in rows {
        print_row(&row.values(), &widths);
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
