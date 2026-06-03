use super::InventoryMapper;
use crate::inventory::source::SourceRecord;
use crate::inventory::source::{ChassisData, SystemData};
use crate::inventory::types::{
    ChassisInventory, ChassisLocation, ChassisSerial, DeviceInventory, DomainError,
    FirmwareInventory, INVENTORY_SCHEMA_VERSION, InventorySourceInfo, ManagerInventory, NodeInfo,
    NodeInventory, SystemUuid,
};

#[derive(Default)]
pub struct RedfishMapper;

impl InventoryMapper for RedfishMapper {
    fn map(&self, record: SourceRecord) -> NodeInventory {
        let target = record.target;
        let collected_at = record.collected_at;
        match record.result {
            Ok(report) => NodeInventory {
                schema_version: INVENTORY_SCHEMA_VERSION.to_string(),
                collected_at,
                source: InventorySourceInfo {
                    source_type: "redfish".to_string(),
                    target: target.name,
                    address: target.address,
                    redfish_version: report.service_root.redfish_version,
                    service_uuid: report.service_root.uuid,
                    vendor: report.service_root.vendor.or(report.service_root.product),
                },
                node: select_primary_system(&report.systems).map(map_node),
                manager: report
                    .managers
                    .into_iter()
                    .next()
                    .map(|manager| ManagerInventory {
                        id: manager.id,
                        odata_id: manager.odata_id,
                        uuid: manager.uuid,
                        manager_type: manager.manager_type,
                        model: manager.model,
                        firmware_version: manager.firmware_version,
                        status_state: manager.status_state,
                        health: manager.health,
                    }),
                chassis: report.chassis.into_iter().map(map_chassis).collect(),
                devices: report.devices.into_iter().map(map_device).collect(),
                firmware: report.firmware.into_iter().map(map_firmware).collect(),
                domain_errors: report
                    .errors
                    .into_iter()
                    .map(|error| DomainError {
                        domain: error.domain,
                        resource: error.resource,
                        message: error.message,
                    })
                    .collect(),
                target_error: None,
            },
            Err(error) => NodeInventory {
                schema_version: INVENTORY_SCHEMA_VERSION.to_string(),
                collected_at,
                source: InventorySourceInfo {
                    source_type: "redfish".to_string(),
                    target: target.name,
                    address: target.address,
                    redfish_version: None,
                    service_uuid: None,
                    vendor: None,
                },
                node: None,
                manager: None,
                chassis: Vec::new(),
                devices: Vec::new(),
                firmware: Vec::new(),
                domain_errors: Vec::new(),
                target_error: Some(error),
            },
        }
    }
}

fn select_primary_system(systems: &[SystemData]) -> Option<SystemData> {
    if systems.is_empty() {
        return None;
    }
    for predicate in [
        has_valid_uuid_and_bios as fn(&SystemData) -> bool,
        has_valid_uuid,
        has_bios,
        has_serial,
    ] {
        if let Some(system) = systems.iter().find(|system| predicate(system)) {
            return Some(system.clone());
        }
    }
    systems.first().cloned()
}

fn has_valid_uuid_and_bios(system: &SystemData) -> bool {
    has_valid_uuid(system) && has_bios(system)
}

fn has_valid_uuid(system: &SystemData) -> bool {
    system.uuid.as_deref().and_then(SystemUuid::parse).is_some()
}

fn has_bios(system: &SystemData) -> bool {
    system
        .bios_version
        .as_deref()
        .is_some_and(|value| !value.trim().is_empty())
}

fn has_serial(system: &SystemData) -> bool {
    system
        .serial_number
        .as_deref()
        .is_some_and(|value| !value.trim().is_empty())
}

fn map_node(system: SystemData) -> NodeInfo {
    NodeInfo {
        system_id: system.id,
        odata_id: system.odata_id,
        system_uuid: system.uuid.as_deref().and_then(SystemUuid::parse),
        manufacturer: system.manufacturer,
        model: system.model,
        sku: system.sku,
        serial_number: system.serial_number,
        bios_version: system.bios_version,
        host_name: system.host_name,
        asset_tag: system.asset_tag,
        power_state: system.power_state.unwrap_or_default(),
        status_state: system.status_state,
        health: system.health,
        cpu_count: system.cpu_count,
        cpu_core_count: system.cpu_core_count,
        cpu_model: system.cpu_model,
        memory_gib: system.memory_gib,
    }
}

fn map_chassis(chassis: ChassisData) -> ChassisInventory {
    ChassisInventory {
        id: chassis.id,
        odata_id: chassis.odata_id,
        chassis_type: chassis.chassis_type,
        manufacturer: chassis.manufacturer,
        model: chassis.model,
        sku: chassis.sku,
        serial_number: chassis
            .serial_number
            .as_deref()
            .and_then(ChassisSerial::parse),
        part_number: chassis.part_number,
        asset_tag: chassis.asset_tag,
        power_state: chassis.power_state,
        status_state: chassis.status_state,
        health: chassis.health,
        location: chassis.location.map(|loc| ChassisLocation {
            service_label: loc.service_label,
            rack: loc.rack,
            row: loc.row,
            room: loc.room,
            rack_offset: loc.rack_offset,
        }),
    }
}

fn map_device(device: crate::inventory::source::DeviceData) -> DeviceInventory {
    DeviceInventory {
        id: device.id,
        odata_id: device.odata_id,
        uuid: device.uuid,
        source: device.source,
        parent_type: device.parent_type,
        parent_id: device.parent_id,
        device_type: device.device_type,
        manufacturer: device.manufacturer,
        model: device.model,
        sku: device.sku,
        serial_number: device.serial_number,
        part_number: device.part_number,
        firmware_version: device.firmware_version,
        status_state: device.status_state,
        health: device.health,
    }
}

fn map_firmware(firmware: crate::inventory::source::FirmwareData) -> FirmwareInventory {
    FirmwareInventory {
        service_id: firmware.service_id,
        id: firmware.id,
        odata_id: firmware.odata_id,
        name: firmware.name,
        version: firmware.version,
        release_date: firmware.release_date,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::Target;
    use crate::inventory::source::{
        ChassisData, DeviceData, FirmwareData, InventoryErrorData, ManagerData, RedfishReport,
        ServiceRootData, SourceRecord, SystemData,
    };

    #[test]
    fn prefers_system_with_valid_uuid_and_bios() {
        let inventory = RedfishMapper.map(SourceRecord {
            target: target(),
            collected_at: collected_at(),
            result: Ok(RedfishReport {
                service_root: ServiceRootData::default(),
                managers: Vec::new(),
                systems: vec![
                    SystemData {
                        id: "BMC".to_string(),
                        odata_id: None,
                        uuid: None,
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
                        cpu_count: None,
                        cpu_core_count: None,
                        cpu_model: None,
                        memory_gib: None,
                    },
                    SystemData {
                        id: "System".to_string(),
                        odata_id: Some("/redfish/v1/Systems/System".to_string()),
                        uuid: Some("9AEE71E4-6E80-11EE-A8AC-74D4DD550187".to_string()),
                        manufacturer: Some("NVIDIA".to_string()),
                        model: Some("GB200".to_string()),
                        sku: None,
                        serial_number: Some("SN123".to_string()),
                        bios_version: Some("1.0".to_string()),
                        host_name: None,
                        asset_tag: None,
                        power_state: Some("on".to_string()),
                        status_state: Some("enabled".to_string()),
                        health: Some("ok".to_string()),
                        cpu_count: Some(1),
                        cpu_core_count: Some(72),
                        cpu_model: None,
                        memory_gib: Some(480.0),
                    },
                ],
                chassis: Vec::new(),
                devices: Vec::new(),
                firmware: Vec::new(),
                errors: Vec::new(),
            }),
        });

        let node = inventory.node.expect("node");
        assert_eq!(node.system_id, "System");
        assert_eq!(node.odata_id.as_deref(), Some("/redfish/v1/Systems/System"));
        assert_eq!(node.status_state.as_deref(), Some("enabled"));
        assert_eq!(
            node.system_uuid.expect("uuid").to_string(),
            "9aee71e4-6e80-11ee-a8ac-74d4dd550187"
        );
    }

    #[test]
    fn trims_and_validates_chassis_serials() {
        let inventory = RedfishMapper.map(SourceRecord {
            target: target(),
            collected_at: collected_at(),
            result: Ok(RedfishReport {
                service_root: ServiceRootData::default(),
                managers: Vec::new(),
                systems: Vec::new(),
                chassis: vec![
                    ChassisData {
                        id: "BMC_0".to_string(),
                        odata_id: None,
                        chassis_type: Some("module".to_string()),
                        manufacturer: None,
                        model: None,
                        sku: None,
                        serial_number: Some("  QTWS7G0241600218  ".to_string()),
                        part_number: None,
                        asset_tag: None,
                        power_state: None,
                        status_state: None,
                        health: None,
                        location: None,
                    },
                    ChassisData {
                        id: "Baseboard_0".to_string(),
                        odata_id: None,
                        chassis_type: Some("board".to_string()),
                        manufacturer: None,
                        model: None,
                        sku: None,
                        serial_number: Some(" ".to_string()),
                        part_number: None,
                        asset_tag: None,
                        power_state: None,
                        status_state: None,
                        health: None,
                        location: None,
                    },
                ],
                devices: Vec::new(),
                firmware: Vec::new(),
                errors: Vec::new(),
            }),
        });

        assert_eq!(
            inventory.chassis[0]
                .serial_number
                .as_ref()
                .expect("serial")
                .to_string(),
            "QTWS7G0241600218"
        );
        assert!(inventory.chassis[1].serial_number.is_none());
    }

    #[test]
    fn preserves_first_system_when_candidates_tie() {
        let inventory = map_report(RedfishReport {
            service_root: ServiceRootData::default(),
            managers: Vec::new(),
            systems: vec![
                system("SystemA")
                    .with_uuid("9AEE71E4-6E80-11EE-A8AC-74D4DD550187")
                    .with_bios("3A21"),
                system("SystemB")
                    .with_uuid("6732BC04-BFDE-11D3-01A8-9C6B007930FB")
                    .with_bios("11.04.NV04"),
            ],
            chassis: Vec::new(),
            devices: Vec::new(),
            firmware: Vec::new(),
            errors: Vec::new(),
        });

        assert_eq!(inventory.node.expect("node").system_id, "SystemA");
    }

    #[test]
    fn falls_back_to_bios_when_uuid_is_malformed() {
        let inventory = map_report(RedfishReport {
            service_root: ServiceRootData::default(),
            managers: Vec::new(),
            systems: vec![
                system("BadUuid").with_uuid("not-a-uuid"),
                system("BiosSystem").with_bios("3A21"),
            ],
            chassis: Vec::new(),
            devices: Vec::new(),
            firmware: Vec::new(),
            errors: Vec::new(),
        });

        let node = inventory.node.expect("node");
        assert_eq!(node.system_id, "BiosSystem");
        assert!(node.system_uuid.is_none());
    }

    #[test]
    fn falls_back_to_first_system_when_no_identity_signal_exists() {
        let inventory = map_report(RedfishReport {
            service_root: ServiceRootData::default(),
            managers: Vec::new(),
            systems: vec![system("SystemA"), system("SystemB")],
            chassis: Vec::new(),
            devices: Vec::new(),
            firmware: Vec::new(),
            errors: Vec::new(),
        });

        assert_eq!(inventory.node.expect("node").system_id, "SystemA");
    }

    #[test]
    fn maps_error_record_without_partial_inventory() {
        let inventory = RedfishMapper.map(SourceRecord {
            target: target(),
            collected_at: collected_at(),
            result: Err("systems collection: timeout".to_string()),
        });

        assert_eq!(inventory.schema_version, INVENTORY_SCHEMA_VERSION);
        assert_eq!(inventory.collected_at, collected_at());
        assert_eq!(inventory.source.target, "node-a");
        assert_eq!(inventory.source.address, "10.0.0.1");
        assert_eq!(
            inventory.target_error.as_deref(),
            Some("systems collection: timeout")
        );
        assert!(inventory.manager.is_none());
        assert!(inventory.node.is_none());
        assert!(inventory.chassis.is_empty());
        assert!(inventory.firmware.is_empty());
        assert!(inventory.domain_errors.is_empty());
    }

    #[test]
    fn maps_top_level_inventory_without_requiring_every_collection() {
        let inventory = map_report(RedfishReport {
            service_root: ServiceRootData {
                redfish_version: Some("1.15.1".to_string()),
                uuid: Some("870155dd-d474-aca8-11ee-6e809af87c0c".to_string()),
                vendor: None,
                product: Some("NVIDIA BMC".to_string()),
            },
            managers: vec![ManagerData {
                id: "BMC_0".to_string(),
                odata_id: Some("/redfish/v1/Managers/BMC_0".to_string()),
                uuid: Some("manager-uuid".to_string()),
                manager_type: Some("bmc".to_string()),
                model: Some("OpenBMC".to_string()),
                firmware_version: Some("3.19.00".to_string()),
                status_state: Some("enabled".to_string()),
                health: Some("ok".to_string()),
            }],
            systems: Vec::new(),
            chassis: Vec::new(),
            devices: Vec::new(),
            firmware: vec![FirmwareData {
                service_id: "FirmwareInventory".to_string(),
                id: "BMCImage1".to_string(),
                odata_id: Some("/redfish/v1/UpdateService/FirmwareInventory/BMCImage1".to_string()),
                name: "BMCImage1".to_string(),
                version: Some("3.19.00".to_string()),
                release_date: None,
            }],
            errors: Vec::new(),
        });

        assert_eq!(inventory.source.redfish_version.as_deref(), Some("1.15.1"));
        assert_eq!(inventory.source.vendor.as_deref(), Some("NVIDIA BMC"));
        let manager = inventory.manager.expect("manager");
        assert_eq!(manager.id, "BMC_0");
        assert_eq!(
            manager.odata_id.as_deref(),
            Some("/redfish/v1/Managers/BMC_0")
        );
        assert_eq!(manager.manager_type.as_deref(), Some("bmc"));
        assert_eq!(manager.status_state.as_deref(), Some("enabled"));
        assert_eq!(manager.health.as_deref(), Some("ok"));
        assert!(inventory.node.is_none());
        assert_eq!(inventory.firmware.len(), 1);
        assert_eq!(
            inventory.firmware[0].odata_id.as_deref(),
            Some("/redfish/v1/UpdateService/FirmwareInventory/BMCImage1")
        );
    }

    #[test]
    fn maps_device_metadata_and_partial_errors() {
        let inventory = map_report(RedfishReport {
            service_root: ServiceRootData::default(),
            managers: Vec::new(),
            systems: Vec::new(),
            chassis: Vec::new(),
            devices: vec![DeviceData {
                id: "GPU0".to_string(),
                odata_id: Some("/redfish/v1/Systems/System/Processors/GPU0".to_string()),
                uuid: Some("22222222-3333-4444-5555-666666666666".to_string()),
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
            firmware: Vec::new(),
            errors: vec![InventoryErrorData {
                domain: "devices.processors".to_string(),
                resource: Some("System".to_string()),
                message: "BMC error".to_string(),
            }],
        });

        let device = inventory.devices.first().expect("device");
        assert_eq!(
            device.odata_id.as_deref(),
            Some("/redfish/v1/Systems/System/Processors/GPU0")
        );
        assert_eq!(device.parent_type.as_deref(), Some("system"));
        assert_eq!(device.status_state.as_deref(), Some("enabled"));
        assert_eq!(inventory.domain_errors.len(), 1);
        assert_eq!(inventory.domain_errors[0].domain, "devices.processors");
    }

    fn map_report(report: RedfishReport) -> NodeInventory {
        RedfishMapper.map(SourceRecord {
            target: target(),
            collected_at: collected_at(),
            result: Ok(report),
        })
    }

    fn collected_at() -> String {
        "2026-01-01T00:00:00Z".to_string()
    }

    fn target() -> Target {
        Target {
            name: "node-a".to_string(),
            address: "10.0.0.1".to_string(),
            username: "user".to_string(),
            password: "pass".to_string(),
        }
    }

    fn system(id: &str) -> SystemData {
        SystemData {
            id: id.to_string(),
            odata_id: None,
            uuid: None,
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
            cpu_count: None,
            cpu_core_count: None,
            cpu_model: None,
            memory_gib: None,
        }
    }

    trait SystemDataTestExt {
        fn with_uuid(self, uuid: &str) -> Self;
        fn with_bios(self, bios: &str) -> Self;
    }

    impl SystemDataTestExt for SystemData {
        fn with_uuid(mut self, uuid: &str) -> Self {
            self.uuid = Some(uuid.to_string());
            self
        }

        fn with_bios(mut self, bios: &str) -> Self {
            self.bios_version = Some(bios.to_string());
            self
        }
    }
}
