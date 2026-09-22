use std::error::Error;
use std::fmt;

use super::InventorySink;
use crate::models::{
    ChassisInventory, FirmwareInventory, InventoryLink, InventoryLocation, InventoryStatus,
    ManagerInventory, NetworkLatencyObservation, NetworkLatencyProtocol, NetworkLatencyStatus,
    NodeInventory, PcieDeviceInventory, ProcessorInventory, SystemInventory,
};

#[derive(Default)]
pub struct TableSink {
    detail: bool,
}

#[derive(Debug)]
struct SummaryRow {
    name: String,
    compute_zone: String,
    node_group: String,
    mac: String,
    address: String,
    host_name: String,
    source_type: String,
    redfish_version: String,
    service_uuid: String,
    vendor: String,
    uuid: String,
    primary_system: String,
    managers: String,
    systems: String,
    chassis: String,
    firmware: String,
    domain_errors: String,
    latency_status: String,
    latency_rtt: String,
    latency_packet_loss: String,
    latency_error: String,
    error: String,
}

#[derive(Debug)]
struct SourceDetailRow {
    source_type: String,
    mac: String,
    address: String,
    host_name: String,
    redfish_version: String,
    service_uuid: String,
    vendor: String,
}

#[derive(Debug)]
struct NetworkLatencyDetailRow {
    protocol: String,
    status: String,
    observed_at: String,
    resolved_address: String,
    attempted_probes: String,
    successful_probes: String,
    minimum_rtt: String,
    median_rtt: String,
    packet_loss: String,
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
    primary: String,
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
struct ProcessorDetailRow {
    system_id: String,
    id: String,
    uuid: String,
    socket: String,
    processor_type: String,
    architecture: String,
    instruction_set: String,
    manufacturer: String,
    model: String,
    serial: String,
    part: String,
    max_speed_mhz: String,
    total_cores: String,
    total_threads: String,
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
    status: String,
    health: String,
    health_rollup: String,
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

fn status_state(status: Option<&InventoryStatus>) -> String {
    status
        .and_then(|status| status.state.clone())
        .unwrap_or_default()
}

fn status_health(status: Option<&InventoryStatus>) -> String {
    status
        .and_then(|status| status.health.clone())
        .unwrap_or_default()
}

fn status_health_rollup(status: Option<&InventoryStatus>) -> String {
    status
        .and_then(|status| status.health_rollup.clone())
        .unwrap_or_default()
}

fn network_latency_protocol(protocol: NetworkLatencyProtocol) -> String {
    match protocol {
        NetworkLatencyProtocol::Icmp => "icmp".to_string(),
    }
}

fn network_latency_status(status: NetworkLatencyStatus) -> String {
    match status {
        NetworkLatencyStatus::Measured => "measured".to_string(),
        NetworkLatencyStatus::Unavailable => "unavailable".to_string(),
    }
}

fn format_rtt(micros: Option<u64>) -> String {
    micros
        .map(|micros| format!("{micros} us"))
        .unwrap_or_default()
}

fn format_packet_loss(percent: f64) -> String {
    format!("{percent:.1}%")
}

fn render_inventory_tables(inventories: &[NodeInventory], detail: bool) -> String {
    let rows = inventories.iter().map(SummaryRow::from).collect::<Vec<_>>();
    let mut out = render_table(
        &[
            "Name",
            "Compute Zone",
            "Node Group",
            "MAC",
            "Address",
            "Host Name",
            "Source",
            "Redfish",
            "Service UUID",
            "Vendor",
            "UUID",
            "Primary System",
            "Managers",
            "Systems",
            "Chassis",
            "Firmware",
            "Domain Errors",
            "Latency Status",
            "Median RTT",
            "Packet Loss",
            "Latency Error",
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
            name: inventory.name.clone().unwrap_or_default(),
            compute_zone: inventory.compute_zone.clone().unwrap_or_default(),
            node_group: inventory.node_group.clone().unwrap_or_default(),
            mac: inventory.source.mac.clone(),
            address: inventory.source.address.clone(),
            host_name: inventory.source.host_name.clone().unwrap_or_default(),
            source_type: inventory.source.source_type.clone(),
            redfish_version: inventory.source.redfish_version.clone().unwrap_or_default(),
            service_uuid: inventory.source.service_uuid.clone().unwrap_or_default(),
            vendor: inventory.source.vendor.clone().unwrap_or_default(),
            uuid: inventory
                .uuid
                .as_ref()
                .map(ToString::to_string)
                .unwrap_or_default(),
            primary_system: inventory.primary_system_id.clone().unwrap_or_default(),
            managers: inventory.managers.len().to_string(),
            systems: inventory.systems.len().to_string(),
            chassis: inventory.chassis.len().to_string(),
            firmware: inventory.firmware.len().to_string(),
            domain_errors: inventory.domain_errors.len().to_string(),
            latency_status: network_latency_status(inventory.network_latency.status),
            latency_rtt: format_rtt(inventory.network_latency.median_rtt_micros),
            latency_packet_loss: format_packet_loss(inventory.network_latency.packet_loss_percent),
            latency_error: inventory.network_latency.error.clone().unwrap_or_default(),
            error: inventory
                .target_error
                .as_ref()
                .map(ToString::to_string)
                .unwrap_or_default(),
        }
    }
}

impl SummaryRow {
    fn values(&self) -> [String; 22] {
        [
            self.name.clone(),
            self.compute_zone.clone(),
            self.node_group.clone(),
            self.mac.clone(),
            self.address.clone(),
            self.host_name.clone(),
            self.source_type.clone(),
            self.redfish_version.clone(),
            self.service_uuid.clone(),
            self.vendor.clone(),
            self.uuid.clone(),
            self.primary_system.clone(),
            self.managers.clone(),
            self.systems.clone(),
            self.chassis.clone(),
            self.firmware.clone(),
            self.domain_errors.clone(),
            self.latency_status.clone(),
            self.latency_rtt.clone(),
            self.latency_packet_loss.clone(),
            self.latency_error.clone(),
            self.error.clone(),
        ]
    }
}

impl From<&NodeInventory> for SourceDetailRow {
    fn from(inventory: &NodeInventory) -> Self {
        Self {
            source_type: inventory.source.source_type.clone(),
            mac: inventory.source.mac.clone(),
            address: inventory.source.address.clone(),
            host_name: inventory.source.host_name.clone().unwrap_or_default(),
            redfish_version: inventory.source.redfish_version.clone().unwrap_or_default(),
            service_uuid: inventory.source.service_uuid.clone().unwrap_or_default(),
            vendor: inventory.source.vendor.clone().unwrap_or_default(),
        }
    }
}

impl SourceDetailRow {
    fn values(&self) -> Vec<[String; 2]> {
        vec![
            ["Source".to_string(), self.source_type.clone()],
            ["MAC".to_string(), self.mac.clone()],
            ["Address".to_string(), self.address.clone()],
            ["Host Name".to_string(), self.host_name.clone()],
            ["Redfish Version".to_string(), self.redfish_version.clone()],
            ["Service UUID".to_string(), self.service_uuid.clone()],
            ["Vendor".to_string(), self.vendor.clone()],
        ]
    }
}

impl From<&NetworkLatencyObservation> for NetworkLatencyDetailRow {
    fn from(observation: &NetworkLatencyObservation) -> Self {
        Self {
            protocol: network_latency_protocol(observation.protocol),
            status: network_latency_status(observation.status),
            observed_at: observation.observed_at.clone(),
            resolved_address: observation.resolved_address.clone(),
            attempted_probes: observation.attempted_probes.to_string(),
            successful_probes: observation.successful_probes.to_string(),
            minimum_rtt: format_rtt(observation.minimum_rtt_micros),
            median_rtt: format_rtt(observation.median_rtt_micros),
            packet_loss: format_packet_loss(observation.packet_loss_percent),
            error: observation.error.clone().unwrap_or_default(),
        }
    }
}

impl NetworkLatencyDetailRow {
    fn values(&self) -> Vec<[String; 2]> {
        vec![
            ["Protocol".to_string(), self.protocol.clone()],
            ["Status".to_string(), self.status.clone()],
            ["Observed At".to_string(), self.observed_at.clone()],
            [
                "Resolved Address".to_string(),
                self.resolved_address.clone(),
            ],
            [
                "Attempted Probes".to_string(),
                self.attempted_probes.clone(),
            ],
            [
                "Successful Probes".to_string(),
                self.successful_probes.clone(),
            ],
            ["Minimum RTT".to_string(), self.minimum_rtt.clone()],
            ["Median RTT".to_string(), self.median_rtt.clone()],
            ["Packet Loss".to_string(), self.packet_loss.clone()],
            ["Error".to_string(), self.error.clone()],
        ]
    }
}

impl From<&ManagerInventory> for ManagerDetailRow {
    fn from(manager: &ManagerInventory) -> Self {
        Self {
            manager_id: manager.id.clone(),
            manager_uuid: manager.uuid.clone().unwrap_or_default(),
            manager_model: manager.model.clone().unwrap_or_default(),
            manager_type: manager.manager_type.clone().unwrap_or_default(),
            manager_firmware: manager.firmware_version.clone().unwrap_or_default(),
            manager_status: status_state(manager.status.as_ref()),
            manager_health: status_health(manager.status.as_ref()),
            manager_health_rollup: status_health_rollup(manager.status.as_ref()),
        }
    }
}

impl ManagerDetailRow {
    fn values(&self) -> [String; 8] {
        [
            self.manager_id.clone(),
            self.manager_uuid.clone(),
            self.manager_model.clone(),
            self.manager_type.clone(),
            self.manager_firmware.clone(),
            self.manager_status.clone(),
            self.manager_health.clone(),
            self.manager_health_rollup.clone(),
        ]
    }
}

impl SystemDetailRow {
    fn from_system(system: &SystemInventory, primary_system_id: Option<&str>) -> Self {
        Self {
            primary: (primary_system_id == Some(system.id.as_str())).to_string(),
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
            status: status_state(system.status.as_ref()),
            health: status_health(system.status.as_ref()),
            health_rollup: status_health_rollup(system.status.as_ref()),
            cpu: format_cpu(system),
            cpu_model: system
                .processor_summary
                .as_ref()
                .and_then(|summary| summary.model.clone())
                .unwrap_or_default(),
            memory_gib: system
                .memory_summary
                .as_ref()
                .and_then(|summary| summary.total_system_memory_gib)
                .map(|memory| memory.to_string())
                .unwrap_or_default(),
            secure_boot_enabled: system
                .secure_boot_enabled
                .map(|enabled| enabled.to_string())
                .unwrap_or_default(),
        }
    }
    fn values(&self) -> [String; 18] {
        [
            self.primary.clone(),
            self.system_id.clone(),
            self.system_uuid.clone(),
            self.manufacturer.clone(),
            self.model.clone(),
            self.sku.clone(),
            self.serial.clone(),
            self.bios.clone(),
            self.host_name.clone(),
            self.asset_tag.clone(),
            self.power.clone(),
            self.status.clone(),
            self.health.clone(),
            self.health_rollup.clone(),
            self.cpu.clone(),
            self.cpu_model.clone(),
            self.memory_gib.clone(),
            self.secure_boot_enabled.clone(),
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
            status: status_state(chassis.status.as_ref()),
            health: status_health(chassis.status.as_ref()),
            health_rollup: status_health_rollup(chassis.status.as_ref()),
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

fn format_location(location: &InventoryLocation) -> String {
    let mut parts = Vec::new();
    if let Some(label) = location
        .part_location
        .as_ref()
        .and_then(|part| part.service_label.as_ref())
    {
        parts.push(label.clone());
    }
    if let Some(context) = &location.part_location_context {
        parts.push(context.clone());
    }
    if let Some(placement) = &location.placement {
        if let Some(rack) = &placement.rack {
            parts.push(format!("rack {rack}"));
        }
        if let Some(offset) = placement.rack_offset {
            parts.push(match placement.rack_offset_units.as_deref() {
                Some(units) => format!("offset {offset} {units}"),
                None => format!("offset {offset}"),
            });
        }
        if let Some(row) = &placement.row {
            parts.push(format!("row {row}"));
        }
        if let Some(room) = &placement.room {
            parts.push(format!("room {room}"));
        }
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
            status: status_state(device.status.as_ref()),
            health: status_health(device.status.as_ref()),
            health_rollup: status_health_rollup(device.status.as_ref()),
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

impl ProcessorDetailRow {
    fn from_processor(system: &SystemInventory, processor: &ProcessorInventory) -> Self {
        Self {
            system_id: system.id.clone(),
            id: processor.id.clone(),
            uuid: processor.uuid.clone().unwrap_or_default(),
            socket: processor.socket.clone().unwrap_or_default(),
            processor_type: processor.processor_type.clone().unwrap_or_default(),
            architecture: processor.processor_architecture.clone().unwrap_or_default(),
            instruction_set: processor.instruction_set.clone().unwrap_or_default(),
            manufacturer: processor.manufacturer.clone().unwrap_or_default(),
            model: processor.model.clone().unwrap_or_default(),
            serial: processor.serial_number.clone().unwrap_or_default(),
            part: processor.part_number.clone().unwrap_or_default(),
            max_speed_mhz: processor
                .max_speed_mhz
                .map(|speed| speed.to_string())
                .unwrap_or_default(),
            total_cores: processor
                .total_cores
                .map(|cores| cores.to_string())
                .unwrap_or_default(),
            total_threads: processor
                .total_threads
                .map(|threads| threads.to_string())
                .unwrap_or_default(),
            status: status_state(processor.status.as_ref()),
            health: status_health(processor.status.as_ref()),
            health_rollup: status_health_rollup(processor.status.as_ref()),
        }
    }

    fn values(&self) -> [String; 17] {
        [
            self.system_id.clone(),
            self.id.clone(),
            self.uuid.clone(),
            self.socket.clone(),
            self.processor_type.clone(),
            self.architecture.clone(),
            self.instruction_set.clone(),
            self.manufacturer.clone(),
            self.model.clone(),
            self.serial.clone(),
            self.part.clone(),
            self.max_speed_mhz.clone(),
            self.total_cores.clone(),
            self.total_threads.clone(),
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
            status: status_state(firmware.status.as_ref()),
            health: status_health(firmware.status.as_ref()),
            health_rollup: status_health_rollup(firmware.status.as_ref()),
        }
    }
}

impl FirmwareDetailRow {
    fn values(&self) -> [String; 8] {
        [
            self.service.clone(),
            self.id.clone(),
            self.name.clone(),
            self.version.clone(),
            self.release_date.clone(),
            self.status.clone(),
            self.health.clone(),
            self.health_rollup.clone(),
        ]
    }
}

fn write_node_details(out: &mut String, inventory: &NodeInventory) {
    out.push('\n');
    out.push_str(&format!("Node: {}\n", inventory.source.address));

    let source_row = SourceDetailRow::from(inventory);
    out.push_str("Source Information\n");
    out.push_str(&render_table(&["Attribute", "Value"], source_row.values()));

    let network_latency_row = NetworkLatencyDetailRow::from(&inventory.network_latency);
    out.push_str("Network Latency\n");
    out.push_str(&render_table(
        &["Attribute", "Value"],
        network_latency_row.values(),
    ));

    let manager_rows = inventory
        .managers
        .iter()
        .map(ManagerDetailRow::from)
        .map(|row| row.values());
    out.push_str("Managers\n");
    out.push_str(&render_table(
        &[
            "ID",
            "UUID",
            "Model",
            "Type",
            "Firmware",
            "Status",
            "Health",
            "Health Rollup",
        ],
        manager_rows,
    ));

    let system_rows = inventory
        .systems
        .iter()
        .map(|system| SystemDetailRow::from_system(system, inventory.primary_system_id.as_deref()))
        .map(|row| row.values());
    out.push_str("Systems\n");
    out.push_str(&render_table(
        &[
            "Primary",
            "ID",
            "UUID",
            "Manufacturer",
            "Model",
            "SKU",
            "Serial",
            "BIOS",
            "Host Name",
            "Asset",
            "Power",
            "Status",
            "Health",
            "Health Rollup",
            "CPU",
            "CPU Model",
            "Mem GiB",
            "Secure Boot",
        ],
        system_rows,
    ));

    let processor_rows = inventory
        .systems
        .iter()
        .flat_map(|system| {
            system
                .processors
                .iter()
                .map(move |processor| ProcessorDetailRow::from_processor(system, processor))
        })
        .map(|row| row.values());
    out.push_str("Processors\n");
    out.push_str(&render_table(
        &[
            "System",
            "ID",
            "UUID",
            "Socket",
            "Type",
            "Arch",
            "Instruction",
            "Manufacturer",
            "Model",
            "Serial",
            "Part",
            "Max MHz",
            "Cores",
            "Threads",
            "Status",
            "Health",
            "Health Rollup",
        ],
        processor_rows,
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
        &[
            "Service",
            "ID",
            "Name",
            "Version",
            "Release Date",
            "Status",
            "Health",
            "Health Rollup",
        ],
        firmware_rows,
    ));

    out.push_str("Links\n");
    out.push_str(&render_table(
        &["Resource Type", "Source", "Relation", "Target"],
        link_detail_rows(inventory),
    ));
}

fn link_detail_rows(inventory: &NodeInventory) -> Vec<[String; 4]> {
    let mut rows = Vec::new();
    for manager in &inventory.managers {
        push_link_detail_rows(
            &mut rows,
            "manager",
            manager.odata_id.as_deref().unwrap_or(&manager.id),
            &manager.links,
        );
    }
    for system in &inventory.systems {
        push_link_detail_rows(
            &mut rows,
            "system",
            system.odata_id.as_deref().unwrap_or(&system.id),
            &system.links,
        );
        for processor in &system.processors {
            push_link_detail_rows(
                &mut rows,
                "processor",
                processor.odata_id.as_deref().unwrap_or(&processor.id),
                &processor.links,
            );
        }
    }
    for chassis in &inventory.chassis {
        push_link_detail_rows(
            &mut rows,
            "chassis",
            chassis.odata_id.as_deref().unwrap_or(&chassis.id),
            &chassis.links,
        );
        for device in &chassis.pcie_devices {
            push_link_detail_rows(
                &mut rows,
                "pcie_device",
                device.odata_id.as_deref().unwrap_or(&device.id),
                &device.links,
            );
        }
    }
    for firmware in &inventory.firmware {
        push_link_detail_rows(
            &mut rows,
            "firmware",
            firmware.odata_id.as_deref().unwrap_or(&firmware.id),
            &firmware.links,
        );
    }
    rows
}

fn push_link_detail_rows(
    rows: &mut Vec<[String; 4]>,
    resource_type: &str,
    source: &str,
    links: &std::collections::BTreeMap<String, InventoryLink>,
) {
    for (relation, targets) in links {
        rows.extend(targets.iter().map(|target| {
            [
                resource_type.to_string(),
                source.to_string(),
                relation.clone(),
                target.odata_id.to_string(),
            ]
        }));
    }
}

fn format_cpu(system: &SystemInventory) -> String {
    match (
        system
            .processor_summary
            .as_ref()
            .and_then(|summary| summary.count),
        system
            .processor_summary
            .as_ref()
            .and_then(|summary| summary.core_count),
    ) {
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
        NetworkLatencyObservation, NetworkLatencyProtocol, NetworkLatencyStatus,
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
    fn location_display_uses_reported_units() {
        let location = InventoryLocation {
            part_location: None,
            part_location_context: None,
            placement: Some(crate::models::Placement {
                rack_offset: Some(3),
                rack_offset_units: Some("OpenU".into()),
                ..Default::default()
            }),
        };
        assert_eq!(format_location(&location), "offset 3 OpenU");
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
        inventory.target_error = Some("timeout".into());

        let row = SummaryRow::from(&inventory);

        assert_eq!(row.name, "node-01");
        assert_eq!(row.compute_zone, "us-west-dc1");
        assert_eq!(row.node_group, "gb200-training");
        assert_eq!(row.source_type, "redfish");
        assert_eq!(row.host_name, "bmc-a.example.com");
        assert_eq!(row.redfish_version, "1.15.1");
        assert_eq!(row.service_uuid, "870155dd-d474-aca8-11ee-6e809af87c0c");
        assert_eq!(row.vendor, "NVIDIA");
        assert_eq!(row.managers, "1");
        assert_eq!(row.uuid, "9aee71e4-6e80-11ee-a8ac-74d4dd550187");
        assert_eq!(row.primary_system, "System");
        assert_eq!(row.systems, "1");
        assert_eq!(row.chassis, "1");
        assert_eq!(row.firmware, "1");
        assert_eq!(row.domain_errors, "0");
        assert_eq!(row.latency_status, "measured");
        assert_eq!(row.latency_rtt, "120 us");
        assert_eq!(row.latency_packet_loss, "20.0%");
        assert_eq!(row.latency_error, "");
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

        assert!(rendered.contains("Address"));
        assert!(rendered.contains("Host Name"));
        assert!(rendered.contains("Source"));
        assert!(rendered.contains("Redfish"));
        assert!(rendered.contains("Service UUID"));
        assert!(rendered.contains("Vendor"));
        assert!(rendered.contains("Latency Status"));
        assert!(rendered.contains("Median RTT"));
        assert!(rendered.contains("Packet Loss"));
        assert!(rendered.contains("Latency Error"));
        assert!(rendered.contains("10.0.0.1"));
        assert!(rendered.contains("bmc-a.example.com"));
        assert!(rendered.contains("redfish"));
        assert!(rendered.contains("1.15.1"));
        assert!(rendered.contains("870155dd-d474-aca8-11ee-6e809af87c0c"));
        assert!(rendered.contains("NVIDIA"));
        assert!(rendered.contains("9aee71e4-6e80-11ee-a8ac-74d4dd550187"));
        assert!(rendered.contains("measured"));
        assert!(rendered.contains("120 us"));
        assert!(rendered.contains("20.0%"));
        assert!(!rendered.contains("Node:"));
        assert!(!rendered.contains("Firmware\n"));
    }

    #[test]
    fn render_inventory_tables_includes_node_chassis_and_firmware_details() {
        let rendered = render_inventory_tables(&[full_inventory()], true);

        assert!(rendered.contains("Node: 10.0.0.1"));
        assert!(rendered.contains("Source Information\n"));
        assert!(rendered.contains("| Host Name       | bmc-a.example.com"));
        assert!(rendered.contains("Redfish Version"));
        assert!(rendered.contains("Service UUID"));
        assert!(rendered.contains("870155dd-d474-aca8-11ee-6e809af87c0c"));
        assert!(rendered.contains("Network Latency\n"));
        assert!(rendered.contains("| Protocol          | icmp"));
        assert!(rendered.contains("| Status            | measured"));
        assert!(rendered.contains("| Minimum RTT       | 80 us"));
        assert!(rendered.contains("| Median RTT        | 120 us"));
        assert!(rendered.contains("| Packet Loss       | 20.0%"));
        assert!(rendered.contains("Managers\n"));
        assert!(rendered.contains("OpenBMC"));
        assert!(rendered.contains("Systems\n"));
        assert!(rendered.contains("Primary"));
        assert!(rendered.contains("System"));
        assert!(rendered.contains("CPU"));
        assert!(rendered.contains("1/72"));
        assert!(rendered.contains("CPU Model"));
        assert!(rendered.contains("Grace CPU"));
        assert!(rendered.contains("Secure Boot"));
        assert!(rendered.contains("Processors\n"));
        assert!(rendered.contains("22222222-3333-4444-5555-666666666666"));
        assert!(rendered.contains("CPU Socket 1"));
        assert!(rendered.contains("CPU-SN123"));
        assert!(rendered.contains("Health Rollup"));
        assert!(rendered.contains("Chassis\n"));
        assert!(rendered.contains("| BMC_0 | module"));
        assert!(rendered.contains("PCIe Devices\n"));
        assert!(rendered.contains("| GPU1"));
        assert!(rendered.contains("Firmware\n"));
        assert!(rendered.contains("| FirmwareInventory | BMC"));
        assert!(rendered.contains("StandbyOffline"));
        assert!(rendered.contains("Critical"));
        assert!(rendered.contains("Links\n"));
        assert!(rendered.contains("ManagerForServers"));
        assert!(rendered.contains("/redfish/v1/Systems/System"));

        let source = rendered.find("Source Information\n").expect("source");
        let network_latency = rendered.find("Network Latency\n").expect("network latency");
        let manager = rendered.find("Managers\n").expect("manager");
        let system = rendered.find("Systems\n").expect("system");
        let processor = rendered.find("Processors\n").expect("processor");
        let chassis = rendered.find("Chassis\n").expect("chassis");
        let pcie = rendered.find("PCIe Devices\n").expect("pcie");
        let firmware = rendered.find("Firmware\n").expect("firmware");
        let links = rendered.find("Links\n").expect("links");
        assert!(source < network_latency);
        assert!(network_latency < manager);
        assert!(manager < system);
        assert!(system < processor);
        assert!(processor < chassis);
        assert!(chassis < pcie);
        assert!(pcie < firmware);
        assert!(firmware < links);
    }

    #[test]
    fn render_inventory_tables_includes_latency_error() {
        let mut inventory = full_inventory();
        inventory.network_latency.status = NetworkLatencyStatus::Unavailable;
        inventory.network_latency.minimum_rtt_micros = None;
        inventory.network_latency.median_rtt_micros = None;
        inventory.network_latency.packet_loss_percent = 100.0;
        inventory.network_latency.error = Some("operation not permitted".to_string());

        let rendered = render_inventory_tables(&[inventory], true);

        assert!(rendered.contains("unavailable"));
        assert!(rendered.contains("100.0%"));
        assert!(rendered.contains("operation not permitted"));
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

    fn inventory_status(state: &str, health: &str, health_rollup: &str) -> InventoryStatus {
        InventoryStatus {
            conditions: Some(Vec::new()),
            health: Some(health.to_string()),
            health_rollup: Some(health_rollup.to_string()),
            state: Some(state.to_string()),
        }
    }

    fn full_inventory() -> NodeInventory {
        NodeInventory {
            schema_version: INVENTORY_SCHEMA_VERSION.to_string(),
            collected_at: "2026-01-01T00:00:00Z".to_string(),
            enrolled_at: None,
            kind: crate::models::TargetKind::Compute,
            name: Some("node-01".to_string()),
            compute_zone: Some("us-west-dc1".to_string()),
            node_group: Some("gb200-training".to_string()),
            uuid: InventoryUuid::parse("9AEE71E4-6E80-11EE-A8AC-74D4DD550187"),
            network_latency: NetworkLatencyObservation {
                protocol: NetworkLatencyProtocol::Icmp,
                status: NetworkLatencyStatus::Measured,
                observed_at: "2026-01-01T00:00:00Z".to_string(),
                resolved_address: "10.0.0.1".to_string(),
                attempted_probes: 5,
                successful_probes: 4,
                minimum_rtt_micros: Some(80),
                median_rtt_micros: Some(120),
                packet_loss_percent: 20.0,
                error: None,
            },
            source: InventorySourceInfo {
                source_type: "redfish".to_string(),
                mac: "00:11:22:33:44:55".to_string(),
                address: "10.0.0.1".to_string(),
                host_name: Some("bmc-a.example.com".to_string()),
                redfish_version: Some("1.15.1".to_string()),
                service_uuid: Some("870155dd-d474-aca8-11ee-6e809af87c0c".to_string()),
                vendor: Some("NVIDIA".to_string()),
            },
            primary_system_id: Some("System".to_string()),
            systems: vec![SystemInventory {
                name: "System".into(),
                description: None,
                id: "System".to_string(),
                odata_id: Some("/redfish/v1/Systems/System".to_string().into()),
                uuid: InventoryUuid::parse("9AEE71E4-6E80-11EE-A8AC-74D4DD550187"),
                manufacturer: Some("NVIDIA".to_string()),
                model: Some("GB200".to_string()),
                sku: Some("SKU-GB200".to_string()),
                serial_number: Some("SN123".to_string()),
                bios_version: Some("3A21".to_string()),
                host_name: Some("node-a.example.com".to_string()),
                asset_tag: Some("rack-12-tray-3".to_string()),
                power_state: Some("on".to_string()),
                status: Some(inventory_status("Enabled", "OK", "Warning")),
                processor_summary: Some(crate::models::ProcessorSummary {
                    count: Some(1),
                    core_count: Some(72),
                    model: Some("Grace CPU".into()),
                    ..Default::default()
                }),
                memory_summary: Some(crate::models::MemorySummary {
                    total_system_memory_gib: Some(480.0),
                    ..Default::default()
                }),
                secure_boot_enabled: Some(true),
                processors: vec![ProcessorInventory {
                    name: "Processor".into(),
                    description: None,
                    id: "CPU1".to_string(),
                    odata_id: Some(
                        "/redfish/v1/Systems/System/Processors/CPU1"
                            .to_string()
                            .into(),
                    ),
                    uuid: Some("22222222-3333-4444-5555-666666666666".to_string()),
                    socket: Some("CPU Socket 1".to_string()),
                    processor_type: Some("cpu".to_string()),
                    processor_architecture: Some("ARM".to_string()),
                    instruction_set: Some("ARM-A64".to_string()),
                    manufacturer: Some("NVIDIA".to_string()),
                    model: Some("Grace CPU".to_string()),
                    serial_number: Some("CPU-SN123".to_string()),
                    part_number: Some("CPU-PN123".to_string()),
                    location: None,
                    nvidia_gpu: None,
                    max_speed_mhz: Some(3600),
                    total_cores: Some(72),
                    total_threads: Some(72),
                    status: Some(inventory_status("Enabled", "OK", "OK")),
                    links: Default::default(),
                }],
                links: Default::default(),
            }],
            managers: vec![ManagerInventory {
                name: "Manager".into(),
                description: None,
                id: "BMC_0".to_string(),
                odata_id: Some("/redfish/v1/Managers/BMC_0".to_string().into()),
                uuid: Some("manager-uuid".to_string()),
                manager_type: Some("bmc".to_string()),
                model: Some("OpenBMC".to_string()),
                firmware_version: Some("3.19.00".to_string()),
                status: Some(inventory_status("Enabled", "OK", "OK")),
                links: std::collections::BTreeMap::from([(
                    "ManagerForServers".into(),
                    vec![crate::models::InventoryLinkReference {
                        odata_id: "/redfish/v1/Systems/System".into(),
                    }],
                )]),
            }],
            chassis: vec![ChassisInventory {
                name: "Chassis".into(),
                description: None,
                id: "BMC_0".to_string(),
                odata_id: Some("/redfish/v1/Chassis/BMC_0".to_string().into()),
                chassis_type: Some("module".to_string()),
                manufacturer: Some("NVIDIA".to_string()),
                model: Some("BMC".to_string()),
                sku: Some("CH-SKU".to_string()),
                serial_number: None,
                part_number: Some("PN123".to_string()),
                asset_tag: Some("rack-12".to_string()),
                power_state: Some("on".to_string()),
                status: Some(inventory_status("Enabled", "OK", "OK")),
                location: None,
                pcie_devices: vec![PcieDeviceInventory {
                    name: "PcieDevice".into(),
                    description: None,
                    id: "GPU1".to_string(),
                    odata_id: Some(
                        "/redfish/v1/Chassis/BMC_0/PCIeDevices/GPU1"
                            .to_string()
                            .into(),
                    ),
                    uuid: None,
                    device_type: Some("single_function".to_string()),
                    manufacturer: Some("NVIDIA".to_string()),
                    model: Some("B200".to_string()),
                    sku: Some("B200-SXM".to_string()),
                    serial_number: Some("GPU-SN456".to_string()),
                    part_number: Some("GPU-PN456".to_string()),
                    firmware_version: Some("96.00.5E.00.02".to_string()),
                    status: Some(inventory_status("Enabled", "OK", "OK")),
                    links: Default::default(),
                }],
                links: Default::default(),
            }],
            firmware: vec![FirmwareInventory {
                description: None,
                related_item: None,
                service_id: "FirmwareInventory".to_string(),
                id: "BMC".to_string(),
                odata_id: Some(
                    "/redfish/v1/UpdateService/FirmwareInventory/BMC"
                        .to_string()
                        .into(),
                ),
                name: "BMC".to_string(),
                version: Some("3.19.00".to_string()),
                release_date: Some("2026-01-01T00:00:00Z".to_string()),
                status: Some(inventory_status("StandbyOffline", "Critical", "Warning")),
                links: Default::default(),
            }],
            domain_errors: Vec::new(),
            target_error: None,
        }
    }

    fn system(cpu_count: Option<i64>, cpu_core_count: Option<i64>) -> SystemInventory {
        SystemInventory {
            name: "System".into(),
            description: None,
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
            status: None,
            processor_summary: Some(crate::models::ProcessorSummary {
                count: cpu_count,
                core_count: cpu_core_count,
                ..Default::default()
            }),
            memory_summary: None,
            secure_boot_enabled: None,
            processors: Vec::new(),
            links: Default::default(),
        }
    }
}
