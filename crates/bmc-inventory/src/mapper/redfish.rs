use async_trait::async_trait;

use super::InventoryMapper;
use super::reverse_dns::{DEFAULT_REVERSE_DNS_TIMEOUT, ReverseDnsResolver, resolve_hostname};
use crate::models::{
    ChassisInventory, ChassisLocation, DomainError, FirmwareInventory, INVENTORY_SCHEMA_VERSION,
    InventorySourceInfo, InventoryUuid, ManagerInventory, NodeInventory, PcieDeviceInventory,
    ProcessorInventory, SystemInventory,
};
use crate::source::SourceRecord;
use crate::source::{ChassisData, SystemData};

const DERIVED_NODE_UUID_KEY_PREFIX: &str = "fleet-intelligence-oob:redfish:";

pub struct RedfishMapper {
    reverse_dns: Option<ReverseDnsResolver>,
}

impl RedfishMapper {
    pub fn new() -> Self {
        let reverse_dns = match ReverseDnsResolver::new() {
            Ok(resolver) => Some(resolver),
            Err(error) => {
                tracing::warn!(
                    %error,
                    "reverse DNS is unavailable; BMC source host names will be omitted"
                );
                None
            }
        };
        Self { reverse_dns }
    }
}

impl Default for RedfishMapper {
    fn default() -> Self {
        Self::new()
    }
}

#[async_trait]
impl InventoryMapper for RedfishMapper {
    async fn map(&self, record: SourceRecord) -> NodeInventory {
        let host_name = resolve_hostname(
            self.reverse_dns.as_ref(),
            record.target.ip,
            DEFAULT_REVERSE_DNS_TIMEOUT,
        )
        .await;
        map_record(record, host_name)
    }
}

fn map_record(record: SourceRecord, host_name: Option<String>) -> NodeInventory {
    let target = record.target;
    let collected_at = record.collected_at;
    match record.result {
        Ok(report) => {
            let source = InventorySourceInfo {
                source_type: "redfish".to_string(),
                mac: target.mac,
                address: target.address,
                host_name,
                redfish_version: report.service_root.redfish_version,
                service_uuid: report.service_root.uuid,
                vendor: report.service_root.vendor.or(report.service_root.product),
            };
            let selected_system = select_primary_system(&report.systems);
            let node_uuid = node_uuid(selected_system.as_ref(), &source);
            let primary_system_id = selected_system.map(|system| system.id);
            let chassis = report
                .chassis
                .into_iter()
                .map(map_chassis)
                .collect::<Vec<_>>();
            NodeInventory {
                schema_version: INVENTORY_SCHEMA_VERSION.to_string(),
                collected_at,
                enrolled_at: None,
                uuid: Some(node_uuid.clone()),
                source,
                primary_system_id: primary_system_id.clone(),
                managers: report.managers.into_iter().map(map_manager).collect(),
                systems: report
                    .systems
                    .into_iter()
                    .map(|system| {
                        let uuid = if primary_system_id.as_ref() == Some(&system.id) {
                            Some(node_uuid.clone())
                        } else {
                            system_uuid(&system)
                        };
                        map_system(system, uuid)
                    })
                    .collect(),
                chassis,
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
            }
        }
        Err(error) => NodeInventory {
            schema_version: INVENTORY_SCHEMA_VERSION.to_string(),
            collected_at,
            enrolled_at: None,
            uuid: None,
            source: InventorySourceInfo {
                source_type: "redfish".to_string(),
                mac: target.mac,
                address: target.address,
                host_name,
                redfish_version: None,
                service_uuid: None,
                vendor: None,
            },
            primary_system_id: None,
            managers: Vec::new(),
            systems: Vec::new(),
            chassis: Vec::new(),
            firmware: Vec::new(),
            domain_errors: Vec::new(),
            target_error: Some(error),
        },
    }
}

fn select_primary_system(systems: &[SystemData]) -> Option<SystemData> {
    if systems.is_empty() {
        return None;
    }
    systems
        .iter()
        .find(|system| has_bios(system))
        .or_else(|| systems.first())
        .cloned()
}

fn has_bios(system: &SystemData) -> bool {
    system
        .bios_version
        .as_deref()
        .is_some_and(|value| !value.trim().is_empty())
}

fn node_uuid(system: Option<&SystemData>, source: &InventorySourceInfo) -> InventoryUuid {
    system
        .and_then(system_uuid)
        .unwrap_or_else(|| derived_node_uuid(source))
}

fn system_uuid(system: &SystemData) -> Option<InventoryUuid> {
    system.uuid.as_deref().and_then(InventoryUuid::parse)
}

fn derived_node_uuid(source: &InventorySourceInfo) -> InventoryUuid {
    let mut key = String::from(DERIVED_NODE_UUID_KEY_PREFIX);
    match normalized_service_uuid(source.service_uuid.as_deref()) {
        Some(service_uuid) => {
            key.push_str("service:");
            key.push_str(&service_uuid);
        }
        None => {
            key.push_str("address:");
            key.push_str(source.address.trim());
        }
    }
    InventoryUuid::from_stable_name(key.as_bytes())
}

fn normalized_service_uuid(service_uuid: Option<&str>) -> Option<String> {
    let service_uuid = service_uuid?.trim();
    if service_uuid.is_empty() {
        return None;
    }
    InventoryUuid::parse(service_uuid)
        .map(|uuid| uuid.to_string())
        .or_else(|| Some(service_uuid.to_string()))
}

fn map_manager(manager: crate::source::ManagerData) -> ManagerInventory {
    ManagerInventory {
        id: manager.id,
        odata_id: manager.odata_id,
        uuid: manager.uuid,
        manager_type: manager.manager_type,
        model: manager.model,
        firmware_version: manager.firmware_version,
        status_state: manager.status_state,
        health: manager.health,
        health_rollup: manager.health_rollup,
    }
}

fn map_system(system: SystemData, uuid: Option<InventoryUuid>) -> SystemInventory {
    SystemInventory {
        id: system.id,
        odata_id: system.odata_id,
        uuid,
        manufacturer: system.manufacturer,
        model: system.model,
        sku: system.sku,
        serial_number: system.serial_number,
        bios_version: system.bios_version,
        host_name: system.host_name,
        asset_tag: system.asset_tag,
        power_state: system.power_state,
        status_state: system.status_state,
        health: system.health,
        health_rollup: system.health_rollup,
        cpu_count: system.cpu_count,
        cpu_core_count: system.cpu_core_count,
        cpu_model: system.cpu_model,
        memory_gib: system.memory_gib,
        secure_boot_enabled: system.secure_boot_enabled,
        processors: system.processors.into_iter().map(map_processor).collect(),
    }
}

fn map_processor(processor: crate::source::ProcessorData) -> ProcessorInventory {
    ProcessorInventory {
        id: processor.id,
        odata_id: processor.odata_id,
        socket: processor.socket,
        processor_type: processor.processor_type,
        processor_architecture: processor.processor_architecture,
        instruction_set: processor.instruction_set,
        manufacturer: processor.manufacturer,
        model: processor.model,
        max_speed_mhz: processor.max_speed_mhz,
        total_cores: processor.total_cores,
        total_threads: processor.total_threads,
        status_state: processor.status_state,
        health: processor.health,
        health_rollup: processor.health_rollup,
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
        serial_number: chassis.serial_number,
        part_number: chassis.part_number,
        asset_tag: chassis.asset_tag,
        power_state: chassis.power_state,
        status_state: chassis.status_state,
        health: chassis.health,
        health_rollup: chassis.health_rollup,
        location: chassis.location.map(|loc| ChassisLocation {
            service_label: loc.service_label,
            rack: loc.rack,
            row: loc.row,
            room: loc.room,
            rack_offset: loc.rack_offset,
        }),
        pcie_devices: chassis
            .pcie_devices
            .into_iter()
            .map(map_pcie_device)
            .collect(),
    }
}

fn map_pcie_device(device: crate::source::PcieDeviceData) -> PcieDeviceInventory {
    PcieDeviceInventory {
        id: device.id,
        odata_id: device.odata_id,
        uuid: device.uuid,
        device_type: device.device_type,
        manufacturer: device.manufacturer,
        model: device.model,
        sku: device.sku,
        serial_number: device.serial_number,
        part_number: device.part_number,
        firmware_version: device.firmware_version,
        status_state: device.status_state,
        health: device.health,
        health_rollup: device.health_rollup,
    }
}

fn map_firmware(firmware: crate::source::FirmwareData) -> FirmwareInventory {
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
    use crate::source::{
        ChassisData, FirmwareData, ManagerData, ProcessorData, RedfishReport, ServiceRootData,
        SourceRecord, SystemData,
    };

    #[test]
    fn maps_resolved_bmc_hostname_to_source_info() {
        let inventory = map_record(
            SourceRecord {
                target: target(),
                collected_at: collected_at(),
                result: Ok(RedfishReport {
                    service_root: ServiceRootData::default(),
                    managers: Vec::new(),
                    systems: Vec::new(),
                    chassis: Vec::new(),
                    firmware: Vec::new(),
                    errors: Vec::new(),
                }),
            },
            Some("bmc-a.example.com".to_string()),
        );

        assert_eq!(
            inventory.source.host_name.as_deref(),
            Some("bmc-a.example.com")
        );
    }

    #[test]
    fn prefers_system_with_bios() {
        let inventory = map(SourceRecord {
            target: target(),
            collected_at: collected_at(),
            result: Ok(RedfishReport {
                service_root: ServiceRootData::default(),
                managers: Vec::new(),
                systems: vec![
                    system("BMC"),
                    system("System")
                        .with_odata_id("/redfish/v1/Systems/System")
                        .with_uuid("9AEE71E4-6E80-11EE-A8AC-74D4DD550187")
                        .with_manufacturer("NVIDIA")
                        .with_model("GB200")
                        .with_serial("SN123")
                        .with_bios("1.0")
                        .with_power_state("on")
                        .with_status_state("Enabled")
                        .with_health("OK")
                        .with_cpu(1, 72)
                        .with_memory_gib(480.0)
                        .with_processor(processor("CPU1")),
                ],
                chassis: Vec::new(),
                firmware: Vec::new(),
                errors: Vec::new(),
            }),
        });

        assert_eq!(
            inventory.uuid.as_ref().expect("node uuid").to_string(),
            "9aee71e4-6e80-11ee-a8ac-74d4dd550187"
        );
        assert_eq!(inventory.primary_system_id.as_deref(), Some("System"));
        assert_eq!(inventory.source.mac, "00:11:22:33:44:55");
        assert_eq!(inventory.systems.len(), 2);
        let system = inventory
            .systems
            .iter()
            .find(|system| system.id == "System")
            .expect("system");
        assert_eq!(system.id, "System");
        assert_eq!(
            system.odata_id.as_deref(),
            Some("/redfish/v1/Systems/System")
        );
        assert_eq!(system.status_state.as_deref(), Some("Enabled"));
        assert_eq!(
            system.uuid.as_ref().expect("uuid").to_string(),
            "9aee71e4-6e80-11ee-a8ac-74d4dd550187"
        );
        assert_eq!(system.processors.len(), 1);
        assert_eq!(system.processors[0].id, "CPU1");
        assert_eq!(system.processors[0].processor_type.as_deref(), Some("cpu"));
        assert_eq!(system.processors[0].total_cores, Some(72));
    }

    #[test]
    fn maps_chassis_serials_without_normalization() {
        let inventory = map(SourceRecord {
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
                        health_rollup: None,
                        location: None,
                        pcie_devices: Vec::new(),
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
                        health_rollup: None,
                        location: None,
                        pcie_devices: Vec::new(),
                    },
                ],
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
            "  QTWS7G0241600218  "
        );
        assert_eq!(inventory.chassis[1].serial_number.as_deref(), Some(" "));
    }

    #[test]
    fn selects_first_bios_system_when_multiple_have_bios() {
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
            errors: Vec::new(),
        });

        assert_eq!(inventory.primary_system_id.as_deref(), Some("SystemA"));
    }

    #[test]
    fn malformed_selected_system_uuid_derives_stable_node_uuid() {
        let report = RedfishReport {
            service_root: ServiceRootData {
                uuid: Some("870155dd-d474-aca8-11ee-6e809af87c0c".to_string()),
                ..ServiceRootData::default()
            },
            managers: Vec::new(),
            systems: vec![
                system("BadUuid").with_uuid("not-a-uuid").with_bios("3A21"),
                system("BiosSystem").with_bios("3A21"),
            ],
            chassis: Vec::new(),
            firmware: Vec::new(),
            errors: Vec::new(),
        };

        let inventory = map_report(report.clone());
        let repeated = map_report(report);

        assert!(inventory.uuid.is_some());
        assert_eq!(inventory.uuid, repeated.uuid);
        let system = inventory
            .systems
            .iter()
            .find(|system| system.id == "BadUuid")
            .expect("system");
        assert_eq!(inventory.primary_system_id.as_deref(), Some("BadUuid"));
        assert_eq!(system.uuid, inventory.uuid);
        assert!(inventory.target_error.is_none());
    }

    #[test]
    fn empty_selected_system_uuid_derives_node_uuid() {
        let inventory = map_report(RedfishReport {
            service_root: ServiceRootData::default(),
            managers: Vec::new(),
            systems: vec![system("EmptyUuid").with_uuid(" ").with_bios("3A21")],
            chassis: Vec::new(),
            firmware: Vec::new(),
            errors: Vec::new(),
        });

        assert!(inventory.uuid.is_some());
        assert_eq!(inventory.primary_system_id.as_deref(), Some("EmptyUuid"));
        assert!(inventory.target_error.is_none());
    }

    #[test]
    fn missing_selected_system_uuid_derives_node_uuid() {
        let inventory = map_report(RedfishReport {
            service_root: ServiceRootData::default(),
            managers: Vec::new(),
            systems: vec![system("SystemA"), system("SystemB")],
            chassis: Vec::new(),
            firmware: Vec::new(),
            errors: Vec::new(),
        });

        assert!(inventory.uuid.is_some());
        assert_eq!(inventory.primary_system_id.as_deref(), Some("SystemA"));
        assert!(inventory.target_error.is_none());
    }

    #[test]
    fn normalizes_service_uuid_for_derived_identity() {
        assert_eq!(
            normalized_service_uuid(Some(" 870155DD-D474-ACA8-11EE-6E809AF87C0C ")).as_deref(),
            Some("870155dd-d474-aca8-11ee-6e809af87c0c")
        );
    }

    #[test]
    fn maps_absent_system_power_state_as_none() {
        let inventory = map_report(RedfishReport {
            service_root: ServiceRootData::default(),
            managers: Vec::new(),
            systems: vec![system("System").with_uuid("9AEE71E4-6E80-11EE-A8AC-74D4DD550187")],
            chassis: Vec::new(),
            firmware: Vec::new(),
            errors: Vec::new(),
        });

        assert!(inventory.systems[0].power_state.is_none());
    }

    #[test]
    fn maps_error_record_without_partial_inventory() {
        let inventory = map(SourceRecord {
            target: target(),
            collected_at: collected_at(),
            result: Err("systems collection: timeout".to_string()),
        });

        assert_eq!(inventory.schema_version, INVENTORY_SCHEMA_VERSION);
        assert_eq!(inventory.collected_at, collected_at());
        assert_eq!(inventory.source.address, "10.0.0.1");
        assert_eq!(inventory.source.mac, "00:11:22:33:44:55");
        assert_eq!(
            inventory.target_error.as_deref(),
            Some("systems collection: timeout")
        );
        assert!(inventory.primary_system_id.is_none());
        assert!(inventory.managers.is_empty());
        assert!(inventory.systems.is_empty());
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
                status_state: Some("Enabled".to_string()),
                health: Some("OK".to_string()),
                health_rollup: Some("OK".to_string()),
            }],
            systems: Vec::new(),
            chassis: Vec::new(),
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
        assert!(inventory.uuid.is_some());
        assert_eq!(inventory.managers.len(), 1);
        let manager = inventory.managers.first().expect("manager");
        assert_eq!(manager.id, "BMC_0");
        assert_eq!(
            manager.odata_id.as_deref(),
            Some("/redfish/v1/Managers/BMC_0")
        );
        assert_eq!(manager.manager_type.as_deref(), Some("bmc"));
        assert_eq!(manager.status_state.as_deref(), Some("Enabled"));
        assert_eq!(manager.health.as_deref(), Some("OK"));
        assert_eq!(manager.health_rollup.as_deref(), Some("OK"));
        assert!(inventory.systems.is_empty());
        assert_eq!(inventory.firmware.len(), 1);
        assert_eq!(
            inventory.firmware[0].odata_id.as_deref(),
            Some("/redfish/v1/UpdateService/FirmwareInventory/BMCImage1")
        );
    }

    fn map_report(report: RedfishReport) -> NodeInventory {
        map(SourceRecord {
            target: target(),
            collected_at: collected_at(),
            result: Ok(report),
        })
    }

    fn map(record: SourceRecord) -> NodeInventory {
        map_record(record, None)
    }

    fn collected_at() -> String {
        "2026-01-01T00:00:00Z".to_string()
    }

    fn target() -> Target {
        Target {
            ip: "10.0.0.1".parse().expect("IP address"),
            port: None,
            mac: "00:11:22:33:44:55".to_string(),
            address: "10.0.0.1".to_string(),
            username: "user".to_string(),
            password: "pass".to_string(),
            labels: Default::default(),
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
            health_rollup: None,
            cpu_count: None,
            cpu_core_count: None,
            cpu_model: None,
            memory_gib: None,
            secure_boot_enabled: None,
            processors: Vec::new(),
        }
    }

    fn processor(id: &str) -> ProcessorData {
        ProcessorData {
            id: id.to_string(),
            odata_id: Some(format!("/redfish/v1/Systems/System/Processors/{id}")),
            socket: Some("CPU Socket 1".to_string()),
            processor_type: Some("cpu".to_string()),
            processor_architecture: Some("ARM".to_string()),
            instruction_set: Some("ARM-A64".to_string()),
            manufacturer: Some("NVIDIA".to_string()),
            model: Some("Grace".to_string()),
            max_speed_mhz: Some(3600),
            total_cores: Some(72),
            total_threads: Some(72),
            status_state: Some("Enabled".to_string()),
            health: Some("OK".to_string()),
            health_rollup: Some("OK".to_string()),
        }
    }

    trait SystemDataTestExt {
        fn with_uuid(self, uuid: &str) -> Self;
        fn with_odata_id(self, odata_id: &str) -> Self;
        fn with_manufacturer(self, manufacturer: &str) -> Self;
        fn with_model(self, model: &str) -> Self;
        fn with_serial(self, serial: &str) -> Self;
        fn with_bios(self, bios: &str) -> Self;
        fn with_power_state(self, power_state: &str) -> Self;
        fn with_status_state(self, status_state: &str) -> Self;
        fn with_health(self, health: &str) -> Self;
        fn with_cpu(self, count: i64, cores: i64) -> Self;
        fn with_memory_gib(self, memory_gib: f64) -> Self;
        fn with_processor(self, processor: ProcessorData) -> Self;
    }

    impl SystemDataTestExt for SystemData {
        fn with_uuid(mut self, uuid: &str) -> Self {
            self.uuid = Some(uuid.to_string());
            self
        }

        fn with_odata_id(mut self, odata_id: &str) -> Self {
            self.odata_id = Some(odata_id.to_string());
            self
        }

        fn with_manufacturer(mut self, manufacturer: &str) -> Self {
            self.manufacturer = Some(manufacturer.to_string());
            self
        }

        fn with_model(mut self, model: &str) -> Self {
            self.model = Some(model.to_string());
            self
        }

        fn with_serial(mut self, serial: &str) -> Self {
            self.serial_number = Some(serial.to_string());
            self
        }

        fn with_bios(mut self, bios: &str) -> Self {
            self.bios_version = Some(bios.to_string());
            self
        }

        fn with_power_state(mut self, power_state: &str) -> Self {
            self.power_state = Some(power_state.to_string());
            self
        }

        fn with_status_state(mut self, status_state: &str) -> Self {
            self.status_state = Some(status_state.to_string());
            self
        }

        fn with_health(mut self, health: &str) -> Self {
            self.health = Some(health.to_string());
            self
        }

        fn with_cpu(mut self, count: i64, cores: i64) -> Self {
            self.cpu_count = Some(count);
            self.cpu_core_count = Some(cores);
            self
        }

        fn with_memory_gib(mut self, memory_gib: f64) -> Self {
            self.memory_gib = Some(memory_gib);
            self
        }

        fn with_processor(mut self, processor: ProcessorData) -> Self {
            self.processors.push(processor);
            self
        }
    }
}
