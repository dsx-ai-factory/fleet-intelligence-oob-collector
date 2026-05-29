use super::InventoryMapper;
use crate::inventory::source::SourceRecord;
use crate::inventory::types::{
    ChassisInventory, FirmwareInventory, ManagerInventory, NodeInventory, SystemInventory,
};

#[derive(Default)]
pub struct RedfishMapper;

impl InventoryMapper for RedfishMapper {
    fn map(&self, record: SourceRecord) -> NodeInventory {
        let target = record.target;
        match record.result {
            Ok(report) => NodeInventory {
                target: target.name,
                address: target.address,
                redfish_version: report.service_root.redfish_version,
                service_uuid: report.service_root.uuid,
                vendor: report.service_root.vendor.or(report.service_root.product),
                manager: report
                    .managers
                    .into_iter()
                    .next()
                    .map(|manager| ManagerInventory {
                        id: manager.id,
                        uuid: manager.uuid,
                        manager_type: manager.manager_type,
                        model: manager.model,
                        firmware_version: manager.firmware_version,
                        interface_count: manager.interface_count,
                    }),
                system: report
                    .systems
                    .into_iter()
                    .next()
                    .map(|system| SystemInventory {
                        id: system.id,
                        uuid: system.uuid,
                        manufacturer: system.manufacturer,
                        model: system.model,
                        serial_number: system.serial_number,
                        bios_version: system.bios_version,
                        power_state: system.power_state.unwrap_or_default(),
                        cpu_count: system.cpu_count,
                        cpu_core_count: system.cpu_core_count,
                        cpu_model: system.cpu_model,
                        memory_gib: system.memory_gib,
                        interface_count: system.interface_count,
                        pcie_device_count: system.pcie_device_count,
                    }),
                chassis: report
                    .chassis
                    .into_iter()
                    .map(|chassis| ChassisInventory {
                        id: chassis.id,
                        chassis_type: chassis.chassis_type,
                        manufacturer: chassis.manufacturer,
                        model: chassis.model,
                        serial_number: chassis.serial_number,
                        part_number: chassis.part_number,
                        network_adapter_count: chassis.network_adapter_count,
                    })
                    .collect(),
                firmware: report
                    .firmware
                    .into_iter()
                    .map(|firmware| FirmwareInventory {
                        service_id: firmware.service_id,
                        id: firmware.id,
                        name: firmware.name,
                        version: firmware.version,
                        release_date: firmware.release_date,
                    })
                    .collect(),
                error: None,
            },
            Err(error) => NodeInventory {
                target: target.name,
                address: target.address,
                redfish_version: None,
                service_uuid: None,
                vendor: None,
                manager: None,
                system: None,
                chassis: Vec::new(),
                firmware: Vec::new(),
                error: Some(error),
            },
        }
    }
}
