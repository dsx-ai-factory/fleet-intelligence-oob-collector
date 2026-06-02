use std::error::Error;
use std::fmt;

use super::InventorySink;
use crate::inventory::types::{
    ChassisInventory, ChassisLocation, DeviceInventory, FirmwareInventory, NodeInventory,
};

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
    health: String,
    chassis: String,
    devices: String,
    firmware: String,
    partial_errors: String,
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
    manager_model: String,
    manager_type: String,
    manager_firmware: String,
    manager_status: String,
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
    cpu: String,
    memory_gib: String,
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
    location: String,
}

#[derive(Debug)]
struct DeviceDetailRow {
    id: String,
    source: String,
    parent_type: String,
    parent: String,
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
            "Target",
            "Address",
            "Redfish",
            "Service UUID",
            "Vendor",
            "Manager",
            "System",
            "System UUID",
            "Health",
            "Chassis",
            "Devices",
            "Firmware",
            "Partial Errors",
            "Error",
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
        let manager = inventory
            .manager
            .as_ref()
            .map(|manager| manager.id.clone())
            .unwrap_or_default();
        let system = inventory
            .node
            .as_ref()
            .map(|node| node.system_id.clone())
            .unwrap_or_default();
        let system_uuid = inventory
            .node
            .as_ref()
            .and_then(|node| node.system_uuid.as_ref().map(ToString::to_string))
            .unwrap_or_default();
        let health = inventory
            .node
            .as_ref()
            .and_then(|node| node.health.clone())
            .unwrap_or_default();

        Self {
            target: inventory.source.target.clone(),
            address: inventory.source.address.clone(),
            redfish: inventory.source.redfish_version.clone().unwrap_or_default(),
            service_uuid: inventory.source.service_uuid.clone().unwrap_or_default(),
            vendor: inventory.source.vendor.clone().unwrap_or_default(),
            manager,
            system,
            system_uuid,
            health,
            chassis: inventory.chassis.len().to_string(),
            devices: inventory.devices.len().to_string(),
            firmware: inventory.firmware.len().to_string(),
            partial_errors: inventory.errors.len().to_string(),
            error: inventory.collection_error.clone().unwrap_or_default(),
        }
    }
}

impl SummaryRow {
    fn values(&self) -> [String; 14] {
        [
            self.target.clone(),
            self.address.clone(),
            self.redfish.clone(),
            self.service_uuid.clone(),
            self.vendor.clone(),
            self.manager.clone(),
            self.system.clone(),
            self.system_uuid.clone(),
            self.health.clone(),
            self.chassis.clone(),
            self.devices.clone(),
            self.firmware.clone(),
            self.partial_errors.clone(),
            self.error.clone(),
        ]
    }
}

impl From<&NodeInventory> for NodeDetailRow {
    fn from(inventory: &NodeInventory) -> Self {
        Self {
            target: inventory.source.target.clone(),
            address: inventory.source.address.clone(),
            vendor: inventory.source.vendor.clone().unwrap_or_default(),
            redfish: inventory.source.redfish_version.clone().unwrap_or_default(),
            service_uuid: inventory.source.service_uuid.clone().unwrap_or_default(),
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
            system_id: inventory
                .node
                .as_ref()
                .map(|node| node.system_id.clone())
                .unwrap_or_default(),
            system_uuid: inventory
                .node
                .as_ref()
                .and_then(|node| node.system_uuid.as_ref().map(ToString::to_string))
                .unwrap_or_default(),
            manufacturer: inventory
                .node
                .as_ref()
                .and_then(|node| node.manufacturer.clone())
                .unwrap_or_default(),
            model: inventory
                .node
                .as_ref()
                .and_then(|node| node.model.clone())
                .unwrap_or_default(),
            sku: inventory
                .node
                .as_ref()
                .and_then(|node| node.sku.clone())
                .unwrap_or_default(),
            serial: inventory
                .node
                .as_ref()
                .and_then(|node| node.serial_number.clone())
                .unwrap_or_default(),
            bios: inventory
                .node
                .as_ref()
                .and_then(|node| node.bios_version.clone())
                .unwrap_or_default(),
            host_name: inventory
                .node
                .as_ref()
                .and_then(|node| node.host_name.clone())
                .unwrap_or_default(),
            asset_tag: inventory
                .node
                .as_ref()
                .and_then(|node| node.asset_tag.clone())
                .unwrap_or_default(),
            power: inventory
                .node
                .as_ref()
                .map(|node| node.power_state.clone())
                .unwrap_or_default(),
            status: inventory
                .node
                .as_ref()
                .and_then(|node| node.status_state.clone())
                .unwrap_or_default(),
            health: inventory
                .node
                .as_ref()
                .and_then(|node| node.health.clone())
                .unwrap_or_default(),
            cpu: inventory.node.as_ref().map(format_cpu).unwrap_or_default(),
            memory_gib: inventory
                .node
                .as_ref()
                .and_then(|node| node.memory_gib)
                .map(|memory| memory.to_string())
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
            ["Manager Model".to_string(), self.manager_model.clone()],
            ["Manager Type".to_string(), self.manager_type.clone()],
            ["BMC FW".to_string(), self.manager_firmware.clone()],
            ["Manager Status".to_string(), self.manager_status.clone()],
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
            ["CPU".to_string(), self.cpu.clone()],
            ["Mem GiB".to_string(), self.memory_gib.clone()],
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
            location: chassis
                .location
                .as_ref()
                .map(format_location)
                .unwrap_or_default(),
        }
    }
}

impl ChassisDetailRow {
    fn values(&self) -> [String; 12] {
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

impl From<&DeviceInventory> for DeviceDetailRow {
    fn from(device: &DeviceInventory) -> Self {
        Self {
            id: device.id.clone(),
            source: device.source.clone(),
            parent_type: device.parent_type.clone().unwrap_or_default(),
            parent: device.parent_id.clone().unwrap_or_default(),
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
        }
    }
}

impl DeviceDetailRow {
    fn values(&self) -> [String; 14] {
        [
            self.id.clone(),
            self.source.clone(),
            self.parent_type.clone(),
            self.parent.clone(),
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
    out.push_str(&format!(
        "Node: {} ({})\n",
        inventory.source.target, inventory.source.address
    ));

    let node_row = NodeDetailRow::from(inventory);
    out.push_str(&render_table(&["Attribute", "Value"], node_row.values()));

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
            "Location",
        ],
        chassis_rows,
    ));

    let device_rows = inventory
        .devices
        .iter()
        .map(DeviceDetailRow::from)
        .map(|row| row.values());
    out.push_str("Devices\n");
    out.push_str(&render_table(
        &[
            "ID",
            "Source",
            "Parent Type",
            "Parent",
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
        ],
        device_rows,
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

fn format_cpu(node: &crate::inventory::types::NodeInfo) -> String {
    match (node.cpu_count, node.cpu_core_count) {
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
    use crate::inventory::types::{
        INVENTORY_SCHEMA_VERSION, InventorySourceInfo, ManagerInventory, NodeInfo, SystemUuid,
    };

    #[test]
    fn render_table_sizes_columns_from_headers_and_rows() {
        let rendered = render_table(
            &["Name", "Value"],
            [
                ["BMC".to_string(), "3.19.00".to_string()],
                ["LongerName".to_string(), "ok".to_string()],
            ],
        );

        assert_eq!(
            rendered,
            concat!(
                "+------------+---------+\n",
                "| Name       | Value   |\n",
                "+------------+---------+\n",
                "| BMC        | 3.19.00 |\n",
                "| LongerName | ok      |\n",
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
        inventory.collection_error = Some("timeout".to_string());

        let row = SummaryRow::from(&inventory);

        assert_eq!(row.chassis, "1");
        assert_eq!(row.firmware, "1");
        assert_eq!(row.partial_errors, "0");
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

        assert!(rendered.contains("| node-a | 10.0.0.1 | 1.15.1"));
        assert!(rendered.contains("| NVIDIA | BMC_0"));
        assert!(!rendered.contains("Node: node-a"));
        assert!(!rendered.contains("Firmware\n"));
    }

    #[test]
    fn render_inventory_tables_includes_node_chassis_and_firmware_details() {
        let rendered = render_inventory_tables(&[full_inventory()], true);

        assert!(rendered.contains("Node: node-a (10.0.0.1)"));
        assert!(rendered.contains("Manager Model"));
        assert!(rendered.contains("OpenBMC"));
        assert!(rendered.contains("CPU"));
        assert!(rendered.contains("1/72"));
        assert!(rendered.contains("Chassis\n"));
        assert!(rendered.contains("| BMC_0 | module"));
        assert!(rendered.contains("Firmware\n"));
        assert!(rendered.contains("| FirmwareInventory | BMC"));
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
            source: InventorySourceInfo {
                source_type: "redfish".to_string(),
                target: "node-a".to_string(),
                address: "10.0.0.1".to_string(),
                redfish_version: Some("1.15.1".to_string()),
                service_uuid: Some("870155dd-d474-aca8-11ee-6e809af87c0c".to_string()),
                vendor: Some("NVIDIA".to_string()),
            },
            node: Some(NodeInfo {
                system_id: "System".to_string(),
                odata_id: Some("/redfish/v1/Systems/System".to_string()),
                system_uuid: SystemUuid::parse("9AEE71E4-6E80-11EE-A8AC-74D4DD550187"),
                manufacturer: Some("NVIDIA".to_string()),
                model: Some("GB200".to_string()),
                sku: Some("SKU-GB200".to_string()),
                serial_number: Some("SN123".to_string()),
                bios_version: Some("3A21".to_string()),
                host_name: Some("node-a.example.com".to_string()),
                asset_tag: Some("rack-12-tray-3".to_string()),
                power_state: "on".to_string(),
                status_state: Some("enabled".to_string()),
                health: Some("ok".to_string()),
                cpu_count: Some(1),
                cpu_core_count: Some(72),
                cpu_model: None,
                memory_gib: Some(480.0),
            }),
            manager: Some(ManagerInventory {
                id: "BMC_0".to_string(),
                odata_id: Some("/redfish/v1/Managers/BMC_0".to_string()),
                uuid: Some("manager-uuid".to_string()),
                manager_type: Some("bmc".to_string()),
                model: Some("OpenBMC".to_string()),
                firmware_version: Some("3.19.00".to_string()),
                status_state: Some("enabled".to_string()),
                health: Some("ok".to_string()),
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
                status_state: Some("enabled".to_string()),
                health: Some("ok".to_string()),
                location: None,
            }],
            devices: vec![DeviceInventory {
                id: "GPU0".to_string(),
                odata_id: Some("/redfish/v1/Systems/System/Processors/GPU0".to_string()),
                uuid: Some("device-uuid".to_string()),
                source: "processor".to_string(),
                parent_type: Some("system".to_string()),
                parent_id: Some("System".to_string()),
                device_type: Some("gpu".to_string()),
                manufacturer: Some("NVIDIA".to_string()),
                model: Some("B200".to_string()),
                sku: None,
                serial_number: Some("GPU-SN123".to_string()),
                part_number: Some("GPU-PN123".to_string()),
                firmware_version: Some("96.00.5E.00.01".to_string()),
                status_state: Some("enabled".to_string()),
                health: Some("ok".to_string()),
            }],
            firmware: vec![FirmwareInventory {
                service_id: "FirmwareInventory".to_string(),
                id: "BMC".to_string(),
                odata_id: Some("/redfish/v1/UpdateService/FirmwareInventory/BMC".to_string()),
                name: "BMC".to_string(),
                version: Some("3.19.00".to_string()),
                release_date: Some("2026-01-01T00:00:00Z".to_string()),
            }],
            errors: Vec::new(),
            collection_error: None,
        }
    }

    fn system(cpu_count: Option<i64>, cpu_core_count: Option<i64>) -> NodeInfo {
        NodeInfo {
            system_id: "System".to_string(),
            odata_id: None,
            system_uuid: SystemUuid::parse("9AEE71E4-6E80-11EE-A8AC-74D4DD550187"),
            manufacturer: None,
            model: None,
            sku: None,
            serial_number: None,
            bios_version: None,
            host_name: None,
            asset_tag: None,
            power_state: String::new(),
            status_state: None,
            health: None,
            cpu_count,
            cpu_core_count,
            cpu_model: None,
            memory_gib: None,
        }
    }
}
