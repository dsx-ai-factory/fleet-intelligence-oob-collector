use model::site_explorer::EndpointExplorationReport;

use super::InventoryMapper;
use crate::inventory::source::SourceRecord;
use crate::inventory::types::{
    ChassisInventory, FirmwareInventory, ManagerInventory, NodeInventory, SystemInventory,
};

#[derive(Default)]
pub struct NicoMapper;

impl InventoryMapper for NicoMapper {
    fn map(&self, record: SourceRecord) -> NodeInventory {
        let target = record.target;
        match record.result {
            Ok(report) => map_report(target.name, target.address, report),
            Err(error) => NodeInventory {
                target: target.name,
                address: target.address,
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

fn map_report(target: String, address: String, report: EndpointExplorationReport) -> NodeInventory {
    NodeInventory {
        target,
        address,
        vendor: report.vendor.map(|vendor| format!("{vendor:?}")),
        manager: report
            .managers
            .into_iter()
            .next()
            .map(|manager| ManagerInventory {
                id: manager.id,
                interface_count: manager.ethernet_interfaces.len(),
            }),
        system: report
            .systems
            .into_iter()
            .next()
            .map(|system| SystemInventory {
                id: system.id,
                manufacturer: system.manufacturer,
                model: system.model,
                serial_number: system.serial_number,
                power_state: format!("{:?}", system.power_state),
                interface_count: system.ethernet_interfaces.len(),
                pcie_device_count: system.pcie_devices.len(),
            }),
        chassis: report
            .chassis
            .into_iter()
            .map(|chassis| ChassisInventory {
                id: chassis.id,
                manufacturer: chassis.manufacturer,
                model: chassis.model,
                serial_number: chassis.serial_number,
                network_adapter_count: chassis.network_adapters.len(),
            })
            .collect(),
        firmware: report
            .service
            .into_iter()
            .flat_map(|service| {
                let service_id = service.id;
                service
                    .inventories
                    .into_iter()
                    .map(move |inventory| FirmwareInventory {
                        service_id: service_id.clone(),
                        id: inventory.id,
                        version: inventory.version,
                        release_date: inventory.release_date,
                    })
            })
            .collect(),
        error: None,
    }
}
