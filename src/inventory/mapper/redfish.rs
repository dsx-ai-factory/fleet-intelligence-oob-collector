use super::InventoryMapper;
use crate::inventory::source::SourceRecord;
use crate::inventory::source::{ChassisData, SystemData};
use crate::inventory::types::{
    ChassisInventory, ChassisSerial, FirmwareInventory, ManagerInventory, NodeInventory,
    SystemInventory, SystemUuid,
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
                system: select_primary_system(&report.systems).map(map_system),
                chassis: report.chassis.into_iter().map(map_chassis).collect(),
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

fn map_system(system: SystemData) -> SystemInventory {
    SystemInventory {
        id: system.id,
        uuid: system.uuid.as_deref().and_then(SystemUuid::parse),
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
    }
}

fn map_chassis(chassis: ChassisData) -> ChassisInventory {
    ChassisInventory {
        id: chassis.id,
        chassis_type: chassis.chassis_type,
        manufacturer: chassis.manufacturer,
        model: chassis.model,
        serial_number: chassis
            .serial_number
            .as_deref()
            .and_then(ChassisSerial::parse),
        part_number: chassis.part_number,
        network_adapter_count: chassis.network_adapter_count,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::Target;
    use crate::inventory::source::{
        ChassisData, FirmwareData, ManagerData, RedfishReport, ServiceRootData, SourceRecord,
        SystemData,
    };

    #[test]
    fn prefers_system_with_valid_uuid_and_bios() {
        let inventory = RedfishMapper.map(SourceRecord {
            target: target(),
            result: Ok(RedfishReport {
                service_root: ServiceRootData::default(),
                managers: Vec::new(),
                systems: vec![
                    SystemData {
                        id: "BMC".to_string(),
                        uuid: None,
                        manufacturer: None,
                        model: None,
                        serial_number: None,
                        bios_version: None,
                        power_state: None,
                        cpu_count: None,
                        cpu_core_count: None,
                        cpu_model: None,
                        memory_gib: None,
                        interface_count: 0,
                        pcie_device_count: 0,
                    },
                    SystemData {
                        id: "System".to_string(),
                        uuid: Some("9AEE71E4-6E80-11EE-A8AC-74D4DD550187".to_string()),
                        manufacturer: Some("NVIDIA".to_string()),
                        model: Some("GB200".to_string()),
                        serial_number: Some("SN123".to_string()),
                        bios_version: Some("1.0".to_string()),
                        power_state: Some("on".to_string()),
                        cpu_count: Some(1),
                        cpu_core_count: Some(72),
                        cpu_model: None,
                        memory_gib: Some(480.0),
                        interface_count: 2,
                        pcie_device_count: 0,
                    },
                ],
                chassis: Vec::new(),
                firmware: Vec::new(),
            }),
        });

        let system = inventory.system.expect("system");
        assert_eq!(system.id, "System");
        assert_eq!(
            system.uuid.expect("uuid").to_string(),
            "9aee71e4-6e80-11ee-a8ac-74d4dd550187"
        );
    }

    #[test]
    fn trims_and_validates_chassis_serials() {
        let inventory = RedfishMapper.map(SourceRecord {
            target: target(),
            result: Ok(RedfishReport {
                service_root: ServiceRootData::default(),
                managers: Vec::new(),
                systems: Vec::new(),
                chassis: vec![
                    ChassisData {
                        id: "BMC_0".to_string(),
                        chassis_type: Some("module".to_string()),
                        manufacturer: None,
                        model: None,
                        serial_number: Some("  QTWS7G0241600218  ".to_string()),
                        part_number: None,
                        network_adapter_count: 0,
                    },
                    ChassisData {
                        id: "Baseboard_0".to_string(),
                        chassis_type: Some("board".to_string()),
                        manufacturer: None,
                        model: None,
                        serial_number: Some(" ".to_string()),
                        part_number: None,
                        network_adapter_count: 0,
                    },
                ],
                firmware: Vec::new(),
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
            firmware: Vec::new(),
        });

        assert_eq!(inventory.system.expect("system").id, "SystemA");
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
            firmware: Vec::new(),
        });

        let system = inventory.system.expect("system");
        assert_eq!(system.id, "BiosSystem");
        assert!(system.uuid.is_none());
    }

    #[test]
    fn falls_back_to_first_system_when_no_identity_signal_exists() {
        let inventory = map_report(RedfishReport {
            service_root: ServiceRootData::default(),
            managers: Vec::new(),
            systems: vec![system("SystemA"), system("SystemB")],
            chassis: Vec::new(),
            firmware: Vec::new(),
        });

        assert_eq!(inventory.system.expect("system").id, "SystemA");
    }

    #[test]
    fn maps_error_record_without_partial_inventory() {
        let inventory = RedfishMapper.map(SourceRecord {
            target: target(),
            result: Err("systems collection: timeout".to_string()),
        });

        assert_eq!(inventory.target, "node-a");
        assert_eq!(inventory.address, "10.0.0.1");
        assert_eq!(
            inventory.error.as_deref(),
            Some("systems collection: timeout")
        );
        assert!(inventory.manager.is_none());
        assert!(inventory.system.is_none());
        assert!(inventory.chassis.is_empty());
        assert!(inventory.firmware.is_empty());
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
                uuid: Some("manager-uuid".to_string()),
                manager_type: Some("bmc".to_string()),
                model: Some("OpenBMC".to_string()),
                firmware_version: Some("3.19.00".to_string()),
                interface_count: 2,
            }],
            systems: Vec::new(),
            chassis: Vec::new(),
            firmware: vec![FirmwareData {
                service_id: "FirmwareInventory".to_string(),
                id: "BMCImage1".to_string(),
                name: "BMCImage1".to_string(),
                version: Some("3.19.00".to_string()),
                release_date: None,
            }],
        });

        assert_eq!(inventory.redfish_version.as_deref(), Some("1.15.1"));
        assert_eq!(inventory.vendor.as_deref(), Some("NVIDIA BMC"));
        assert_eq!(inventory.manager.expect("manager").id, "BMC_0");
        assert!(inventory.system.is_none());
        assert_eq!(inventory.firmware.len(), 1);
    }

    fn map_report(report: RedfishReport) -> NodeInventory {
        RedfishMapper.map(SourceRecord {
            target: target(),
            result: Ok(report),
        })
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
            uuid: None,
            manufacturer: None,
            model: None,
            serial_number: None,
            bios_version: None,
            power_state: None,
            cpu_count: None,
            cpu_core_count: None,
            cpu_model: None,
            memory_gib: None,
            interface_count: 0,
            pcie_device_count: 0,
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
