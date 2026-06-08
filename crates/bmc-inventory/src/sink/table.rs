use std::error::Error;
use std::fmt;

use super::InventorySink;
use crate::models::{
    ChassisInventory, ChassisLocation, FirmwareInventory, NodeInventory, PcieDeviceInventory,
    SystemInventory,
};

#[derive(Default)]
pub struct TableSink {
    detail: bool,
}

#[derive(Debug)]
struct SummaryRow {
    address: String,
    uuid: String,
    managers: String,
    systems: String,
    chassis: String,
    firmware: String,
    domain_errors: String,
    error: String,
}

#[derive(Debug)]
struct ManagerDetailRow {
    manager_id: String,
    manager_uuid: String,
    manager_model: String,
    manager_type: String,
    manager_firmware: String,
    manager_status: String,
    manager_health: String,
    manager_health_rollup: String,
}

#[derive(Debug)]
struct SystemDetailRow {
    system_id: String,
    system_uuid: String,
    manufacturer: String,
    model: String,
    sku: String,
    serial: String,
    bios: String,
    host_name: String,
    asset_tag: String,
    power: String,
    status: String,
    health: String,
    health_rollup: String,
    cpu: String,
    cpu_model: String,
    memory_gib: String,
    secure_boot_enabled: String,
}

#[derive(Debug)]
struct ChassisDetailRow {
    id: String,
    chassis_type: String,
    manufacturer: String,
    model: String,
    sku: String,
    serial: String,
    part: String,
    asset_tag: String,
    power: String,
    status: String,
    health: String,
    health_rollup: String,
    location: String,
}

#[derive(Debug)]
struct DeviceDetailRow {
    id: String,
    source: String,
    uuid: String,
    device_type: String,
    manufacturer: String,
    model: String,
    sku: String,
    serial: String,
    part: String,
    firmware: String,
    status: String,
    health: String,
    health_rollup: String,
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
        print!("{}", render_inventory_tables(inventories, self.detail));
        Ok(())
    }
}

fn render_inventory_tables(inventories: &[NodeInventory], detail: bool) -> String {
    let rows = inventories.iter().map(SummaryRow::from).collect::<Vec<_>>();
    let mut out = render_table(
        &[
            "Address",
            "UUID",
            "Managers",
            "Systems",
            "Chassis",
            "Firmware",
            "Domain Errors",
            "Target Error",
        ],
        rows.iter().map(SummaryRow::values),
    );

    if detail {
        for inventory in inventories {
            write_node_details(&mut out, inventory);
        }
    }

    out
}

impl From<&NodeInventory> for SummaryRow {
    fn from(inventory: &NodeInventory) -> Self {
        Self {
            address: inventory.source.address.clone(),
            uuid: inventory
                .uuid
                .as_ref()
                .map(ToString::to_string)
                .unwrap_or_default(),
            managers: usize::from(inventory.manager.is_some()).to_string(),
            systems: usize::from(inventory.system.is_some()).to_string(),
            chassis: inventory.chassis.len().to_string(),
            firmware: inventory.firmware.len().to_string(),
            domain_errors: inventory.domain_errors.len().to_string(),
            error: inventory.target_error.clone().unwrap_or_default(),
        }
    }
}

impl SummaryRow {
    fn values(&self) -> [String; 8] {
        [
            self.address.clone(),
            self.uuid.clone(),
            self.managers.clone(),
            self.systems.clone(),
            self.chassis.clone(),
            self.firmware.clone(),
            self.domain_errors.clone(),
            self.error.clone(),
        ]
    }
}

impl From<&NodeInventory> for ManagerDetailRow {
    fn from(inventory: &NodeInventory) -> Self {
        Self {
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
            manager_model: inventory
                .manager
                .as_ref()
                .and_then(|manager| manager.model.clone())
                .unwrap_or_default(),
            manager_type: inventory
                .manager
                .as_ref()
                .and_then(|manager| manager.manager_type.clone())
                .unwrap_or_default(),
            manager_firmware: inventory
                .manager
                .as_ref()
                .and_then(|manager| manager.firmware_version.clone())
                .unwrap_or_default(),
            manager_status: inventory
                .manager
                .as_ref()
                .and_then(|manager| manager.status_state.clone())
                .unwrap_or_default(),
            manager_health: inventory
                .manager
                .as_ref()
                .and_then(|manager| manager.health.clone())
                .unwrap_or_default(),
            manager_health_rollup: inventory
                .manager
                .as_ref()
                .and_then(|manager| manager.health_rollup.clone())
                .unwrap_or_default(),
        }
    }
}

impl ManagerDetailRow {
    fn values(&self) -> Vec<[String; 2]> {
        vec![
            ["Manager".to_string(), self.manager_id.clone()],
            ["Manager UUID".to_string(), self.manager_uuid.clone()],
            ["Manager Model".to_string(), self.manager_model.clone()],
            ["Manager Type".to_string(), self.manager_type.clone()],
            ["BMC FW".to_string(), self.manager_firmware.clone()],
            ["Manager Status".to_string(), self.manager_status.clone()],
            ["Manager Health".to_string(), self.manager_health.clone()],
            [
                "Manager Health Rollup".to_string(),
                self.manager_health_rollup.clone(),
            ],
        ]
    }
}

impl From<&SystemInventory> for SystemDetailRow {
    fn from(system: &SystemInventory) -> Self {
        Self {
            system_id: system.id.clone(),
            system_uuid: system
                .uuid
                .as_ref()
                .map(ToString::to_string)
                .unwrap_or_default(),
            manufacturer: system.manufacturer.clone().unwrap_or_default(),
            model: system.model.clone().unwrap_or_default(),
            sku: system.sku.clone().unwrap_or_default(),
            serial: system.serial_number.clone().unwrap_or_default(),
            bios: system.bios_version.clone().unwrap_or_default(),
            host_name: system.host_name.clone().unwrap_or_default(),
            asset_tag: system.asset_tag.clone().unwrap_or_default(),
            power: system.power_state.clone().unwrap_or_default(),
            status: system.status_state.clone().unwrap_or_default(),
            health: system.health.clone().unwrap_or_default(),
            health_rollup: system.health_rollup.clone().unwrap_or_default(),
            cpu: format_cpu(system),
            cpu_model: system.cpu_model.clone().unwrap_or_default(),
            memory_gib: system
                .memory_gib
                .map(|memory| memory.to_string())
                .unwrap_or_default(),
            secure_boot_enabled: system
                .secure_boot_enabled
                .map(|enabled| enabled.to_string())
                .unwrap_or_default(),
        }
    }
}

impl SystemDetailRow {
    fn values(&self) -> Vec<[String; 2]> {
        vec![
            ["System".to_string(), self.system_id.clone()],
            ["System UUID".to_string(), self.system_uuid.clone()],
            ["Manufacturer".to_string(), self.manufacturer.clone()],
            ["Model".to_string(), self.model.clone()],
            ["SKU".to_string(), self.sku.clone()],
            ["Serial".to_string(), self.serial.clone()],
            ["BIOS".to_string(), self.bios.clone()],
            ["Host Name".to_string(), self.host_name.clone()],
            ["Asset Tag".to_string(), self.asset_tag.clone()],
            ["Power".to_string(), self.power.clone()],
            ["Status".to_string(), self.status.clone()],
            ["Health".to_string(), self.health.clone()],
            ["Health Rollup".to_string(), self.health_rollup.clone()],
            ["CPU".to_string(), self.cpu.clone()],
            ["CPU Model".to_string(), self.cpu_model.clone()],
            ["Mem GiB".to_string(), self.memory_gib.clone()],
            [
                "Secure Boot Enabled".to_string(),
                self.secure_boot_enabled.clone(),
            ],
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
            sku: chassis.sku.clone().unwrap_or_default(),
            serial: chassis
                .serial_number
                .as_ref()
                .map(ToString::to_string)
                .unwrap_or_default(),
            part: chassis.part_number.clone().unwrap_or_default(),
            asset_tag: chassis.asset_tag.clone().unwrap_or_default(),
            power: chassis.power_state.clone().unwrap_or_default(),
            status: chassis.status_state.clone().unwrap_or_default(),
            health: chassis.health.clone().unwrap_or_default(),
            health_rollup: chassis.health_rollup.clone().unwrap_or_default(),
            location: chassis
                .location
                .as_ref()
                .map(format_location)
                .unwrap_or_default(),
        }
    }
}

impl ChassisDetailRow {
    fn values(&self) -> [String; 13] {
        [
            self.id.clone(),
            self.chassis_type.clone(),
            self.manufacturer.clone(),
            self.model.clone(),
            self.sku.clone(),
            self.serial.clone(),
            self.part.clone(),
            self.asset_tag.clone(),
            self.power.clone(),
            self.status.clone(),
            self.health.clone(),
            self.health_rollup.clone(),
            self.location.clone(),
        ]
    }
}

fn format_location(location: &ChassisLocation) -> String {
    let mut parts = Vec::new();
    if let Some(label) = &location.service_label {
        parts.push(label.clone());
    }
    if let Some(rack) = &location.rack {
        match location.rack_offset {
            Some(offset) => parts.push(format!("rack {rack} U{offset}")),
            None => parts.push(format!("rack {rack}")),
        }
    } else if let Some(offset) = location.rack_offset {
        parts.push(format!("U{offset}"));
    }
    if let Some(row) = &location.row {
        parts.push(format!("row {row}"));
    }
    if let Some(room) = &location.room {
        parts.push(format!("room {room}"));
    }
    parts.join(", ")
}

impl DeviceDetailRow {
    fn from_pcie_device(device: &PcieDeviceInventory) -> Self {
        Self {
            id: device.id.clone(),
            source: "pcie_device".to_string(),
            uuid: device.uuid.clone().unwrap_or_default(),
            device_type: device.device_type.clone().unwrap_or_default(),
            manufacturer: device.manufacturer.clone().unwrap_or_default(),
            model: device.model.clone().unwrap_or_default(),
            sku: device.sku.clone().unwrap_or_default(),
            serial: device.serial_number.clone().unwrap_or_default(),
            part: device.part_number.clone().unwrap_or_default(),
            firmware: device.firmware_version.clone().unwrap_or_default(),
            status: device.status_state.clone().unwrap_or_default(),
            health: device.health.clone().unwrap_or_default(),
            health_rollup: device.health_rollup.clone().unwrap_or_default(),
        }
    }

    fn values(&self) -> [String; 13] {
        [
            self.id.clone(),
            self.source.clone(),
            self.uuid.clone(),
            self.device_type.clone(),
            self.manufacturer.clone(),
            self.model.clone(),
            self.sku.clone(),
            self.serial.clone(),
            self.part.clone(),
            self.firmware.clone(),
            self.status.clone(),
            self.health.clone(),
            self.health_rollup.clone(),
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

fn write_node_details(out: &mut String, inventory: &NodeInventory) {
    out.push('\n');
    out.push_str(&format!("Node: {}\n", inventory.source.address));

    let manager_row = ManagerDetailRow::from(inventory);
    out.push_str("Manager Information\n");
    out.push_str(&render_table(&["Attribute", "Value"], manager_row.values()));

    let system_row = inventory.system.as_ref().map(SystemDetailRow::from);
    out.push_str("System Information\n");
    out.push_str(&render_table(
        &["Attribute", "Value"],
        system_row
            .as_ref()
            .map(SystemDetailRow::values)
            .unwrap_or_default(),
    ));

    let chassis_rows = inventory
        .chassis
        .iter()
        .map(ChassisDetailRow::from)
        .map(|row| row.values());
    out.push_str("Chassis\n");
    out.push_str(&render_table(
        &[
            "ID",
            "Type",
            "Manufacturer",
            "Model",
            "SKU",
            "Serial",
            "Part",
            "Asset",
            "Power",
            "Status",
            "Health",
            "Health Rollup",
            "Location",
        ],
        chassis_rows,
    ));

    let pcie_device_rows = inventory
        .chassis
        .iter()
        .flat_map(|chassis| chassis.pcie_devices.iter())
        .map(DeviceDetailRow::from_pcie_device)
        .map(|row| row.values());
    out.push_str("PCIe Devices\n");
    out.push_str(&render_table(
        &[
            "ID",
            "Source",
            "UUID",
            "Type",
            "Manufacturer",
            "Model",
            "SKU",
            "Serial",
            "Part",
            "Firmware",
            "Status",
            "Health",
            "Health Rollup",
        ],
        pcie_device_rows,
    ));

    let firmware_rows = inventory
        .firmware
        .iter()
        .map(FirmwareDetailRow::from)
        .map(|row| row.values());
    out.push_str("Firmware\n");
    out.push_str(&render_table(
        &["Service", "ID", "Name", "Version", "Release Date"],
        firmware_rows,
    ));
}

fn format_cpu(system: &SystemInventory) -> String {
    match (system.cpu_count, system.cpu_core_count) {
        (Some(count), Some(cores)) => format!("{count}/{cores}"),
        (Some(count), None) => count.to_string(),
        (None, Some(cores)) => format!("?/{cores}"),
        (None, None) => String::new(),
    }
}

fn render_table<const N: usize>(
    headers: &[&str; N],
    rows: impl IntoIterator<Item = [String; N]>,
) -> String {
    let rows = rows.into_iter().collect::<Vec<_>>();
    let mut widths = headers.map(str::len);
    for row in &rows {
        for (idx, value) in row.iter().enumerate() {
            widths[idx] = widths[idx].max(value.len());
        }
    }

    let mut out = String::new();
    write_separator(&mut out, &widths);
    write_row(&mut out, headers.as_slice(), &widths);
    write_separator(&mut out, &widths);
    for row in &rows {
        write_row(&mut out, row, &widths);
    }
    write_separator(&mut out, &widths);
    out
}

fn write_separator(out: &mut String, widths: &[usize]) {
    out.push('+');
    for width in widths {
        out.push_str(&"-".repeat(width + 2));
        out.push('+');
    }
    out.push('\n');
}

fn write_row<T: fmt::Display>(out: &mut String, values: &[T], widths: &[usize]) {
    out.push('|');
    for (value, width) in values.iter().zip(widths) {
        out.push_str(&format!(" {value:<width$} |"));
    }
    out.push('\n');
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::models::{
        INVENTORY_SCHEMA_VERSION, InventorySourceInfo, InventoryUuid, ManagerInventory,
        PcieDeviceInventory, SystemInventory,
    };

    #[test]
    fn render_table_sizes_columns_from_headers_and_rows() {
        let rendered = render_table(
            &["Name", "Value"],
            [
                ["BMC".to_string(), "3.19.00".to_string()],
                ["LongerName".to_string(), "x".to_string()],
            ],
        );

        assert_eq!(
            rendered,
            concat!(
                "+------------+---------+\n",
                "| Name       | Value   |\n",
                "+------------+---------+\n",
                "| BMC        | 3.19.00 |\n",
                "| LongerName | x       |\n",
                "+------------+---------+\n",
            )
        );
    }

    #[test]
    fn render_table_handles_empty_rows() {
        let rendered = render_table(&["ID"], std::iter::empty::<[String; 1]>());

        assert_eq!(
            rendered,
            concat!("+----+\n", "| ID |\n", "+----+\n", "+----+\n",)
        );
    }

    #[test]
    fn summary_row_formats_error_and_counts() {
        let mut inventory = full_inventory();
        inventory.target_error = Some("timeout".to_string());

        let row = SummaryRow::from(&inventory);

        assert_eq!(row.managers, "1");
        assert_eq!(row.uuid, "9aee71e4-6e80-11ee-a8ac-74d4dd550187");
        assert_eq!(row.systems, "1");
        assert_eq!(row.chassis, "1");
        assert_eq!(row.firmware, "1");
        assert_eq!(row.domain_errors, "0");
        assert_eq!(row.error, "timeout");
    }

    #[test]
    fn summary_row_leaves_error_empty_on_success() {
        let row = SummaryRow::from(&full_inventory());

        assert_eq!(row.error, "");
    }

    #[test]
    fn render_inventory_tables_includes_summary_only_when_detail_is_false() {
        let rendered = render_inventory_tables(&[full_inventory()], false);

        assert!(rendered.contains("| Address  | UUID                                 | Managers | Systems | Chassis | Firmware"));
        assert!(rendered.contains(
            "| 10.0.0.1 | 9aee71e4-6e80-11ee-a8ac-74d4dd550187 | 1        | 1       | 1       | 1"
        ));
        assert!(!rendered.contains("Node:"));
        assert!(!rendered.contains("Firmware\n"));
    }

    #[test]
    fn render_inventory_tables_includes_node_chassis_and_firmware_details() {
        let rendered = render_inventory_tables(&[full_inventory()], true);

        assert!(rendered.contains("Node: 10.0.0.1"));
        assert!(rendered.contains("Manager Information\n"));
        assert!(rendered.contains("Manager Model"));
        assert!(rendered.contains("OpenBMC"));
        assert!(rendered.contains("System Information\n"));
        assert!(rendered.contains("CPU"));
        assert!(rendered.contains("1/72"));
        assert!(rendered.contains("CPU Model"));
        assert!(rendered.contains("Grace CPU"));
        assert!(rendered.contains("Secure Boot Enabled"));
        assert!(rendered.contains("Health Rollup"));
        assert!(rendered.contains("Chassis\n"));
        assert!(rendered.contains("| BMC_0 | module"));
        assert!(rendered.contains("PCIe Devices\n"));
        assert!(rendered.contains("| GPU1"));
        assert!(rendered.contains("Firmware\n"));
        assert!(rendered.contains("| FirmwareInventory | BMC"));

        let manager = rendered.find("Manager Information\n").expect("manager");
        let system = rendered.find("System Information\n").expect("system");
        let chassis = rendered.find("Chassis\n").expect("chassis");
        let pcie = rendered.find("PCIe Devices\n").expect("pcie");
        let firmware = rendered.find("Firmware\n").expect("firmware");
        assert!(manager < system);
        assert!(system < chassis);
        assert!(chassis < pcie);
        assert!(pcie < firmware);
    }

    #[test]
    fn table_sink_write_accepts_empty_inventory() {
        let sink = TableSink::new(false);

        sink.write(&[]).expect("write");
    }

    #[test]
    fn format_cpu_handles_partial_summary() {
        assert_eq!(format_cpu(&system(Some(1), Some(72))), "1/72");
        assert_eq!(format_cpu(&system(Some(1), None)), "1");
        assert_eq!(format_cpu(&system(None, Some(72))), "?/72");
        assert_eq!(format_cpu(&system(None, None)), "");
    }

    fn full_inventory() -> NodeInventory {
        NodeInventory {
            schema_version: INVENTORY_SCHEMA_VERSION.to_string(),
            collected_at: "2026-01-01T00:00:00Z".to_string(),
            uuid: InventoryUuid::parse("9AEE71E4-6E80-11EE-A8AC-74D4DD550187"),
            source: InventorySourceInfo {
                source_type: "redfish".to_string(),
                address: "10.0.0.1".to_string(),
                redfish_version: Some("1.15.1".to_string()),
                service_uuid: Some("870155dd-d474-aca8-11ee-6e809af87c0c".to_string()),
                vendor: Some("NVIDIA".to_string()),
            },
            system: Some(SystemInventory {
                id: "System".to_string(),
                odata_id: Some("/redfish/v1/Systems/System".to_string()),
                uuid: InventoryUuid::parse("9AEE71E4-6E80-11EE-A8AC-74D4DD550187"),
                manufacturer: Some("NVIDIA".to_string()),
                model: Some("GB200".to_string()),
                sku: Some("SKU-GB200".to_string()),
                serial_number: Some("SN123".to_string()),
                bios_version: Some("3A21".to_string()),
                host_name: Some("node-a.example.com".to_string()),
                asset_tag: Some("rack-12-tray-3".to_string()),
                power_state: Some("on".to_string()),
                status_state: Some("Enabled".to_string()),
                health: Some("OK".to_string()),
                health_rollup: Some("Warning".to_string()),
                cpu_count: Some(1),
                cpu_core_count: Some(72),
                cpu_model: Some("Grace CPU".to_string()),
                memory_gib: Some(480.0),
                secure_boot_enabled: Some(true),
            }),
            manager: Some(ManagerInventory {
                id: "BMC_0".to_string(),
                odata_id: Some("/redfish/v1/Managers/BMC_0".to_string()),
                uuid: Some("manager-uuid".to_string()),
                manager_type: Some("bmc".to_string()),
                model: Some("OpenBMC".to_string()),
                firmware_version: Some("3.19.00".to_string()),
                status_state: Some("Enabled".to_string()),
                health: Some("OK".to_string()),
                health_rollup: Some("OK".to_string()),
            }),
            chassis: vec![ChassisInventory {
                id: "BMC_0".to_string(),
                odata_id: Some("/redfish/v1/Chassis/BMC_0".to_string()),
                chassis_type: Some("module".to_string()),
                manufacturer: Some("NVIDIA".to_string()),
                model: Some("BMC".to_string()),
                sku: Some("CH-SKU".to_string()),
                serial_number: None,
                part_number: Some("PN123".to_string()),
                asset_tag: Some("rack-12".to_string()),
                power_state: Some("on".to_string()),
                status_state: Some("Enabled".to_string()),
                health: Some("OK".to_string()),
                health_rollup: Some("OK".to_string()),
                location: None,
                pcie_devices: vec![PcieDeviceInventory {
                    id: "GPU1".to_string(),
                    odata_id: Some("/redfish/v1/Chassis/BMC_0/PCIeDevices/GPU1".to_string()),
                    uuid: None,
                    device_type: Some("single_function".to_string()),
                    manufacturer: Some("NVIDIA".to_string()),
                    model: Some("B200".to_string()),
                    sku: Some("B200-SXM".to_string()),
                    serial_number: Some("GPU-SN456".to_string()),
                    part_number: Some("GPU-PN456".to_string()),
                    firmware_version: Some("96.00.5E.00.02".to_string()),
                    status_state: Some("Enabled".to_string()),
                    health: Some("OK".to_string()),
                    health_rollup: Some("OK".to_string()),
                }],
            }],
            firmware: vec![FirmwareInventory {
                service_id: "FirmwareInventory".to_string(),
                id: "BMC".to_string(),
                odata_id: Some("/redfish/v1/UpdateService/FirmwareInventory/BMC".to_string()),
                name: "BMC".to_string(),
                version: Some("3.19.00".to_string()),
                release_date: Some("2026-01-01T00:00:00Z".to_string()),
            }],
            domain_errors: Vec::new(),
            target_error: None,
        }
    }

    fn system(cpu_count: Option<i64>, cpu_core_count: Option<i64>) -> SystemInventory {
        SystemInventory {
            id: "System".to_string(),
            odata_id: None,
            uuid: InventoryUuid::parse("9AEE71E4-6E80-11EE-A8AC-74D4DD550187"),
            manufacturer: None,
            model: None,
            sku: None,
            serial_number: None,
            bios_version: None,
            host_name: None,
            asset_tag: None,
            power_state: None,
            status_state: None,
            health: None,
            health_rollup: None,
            cpu_count,
            cpu_core_count,
            cpu_model: None,
            memory_gib: None,
            secure_boot_enabled: None,
        }
    }
}
