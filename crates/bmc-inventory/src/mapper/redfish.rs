use async_trait::async_trait;
use bmc_error::{SanitizedErrorMessage, SanitizedUriReference};

use super::InventoryMapper;
use super::reverse_dns::{DEFAULT_REVERSE_DNS_TIMEOUT, ReverseDnsResolver, resolve_hostname};
use crate::models::{
    ChassisInventory, DomainError, FirmwareInventory, INVENTORY_SCHEMA_VERSION, InventoryCondition,
    InventoryLink, InventoryLinkReference, InventoryLocation, InventoryReference,
    InventorySourceInfo, InventoryStatus, InventoryUuid, ManagerInventory, NodeInventory,
    NvidiaGpuInventory, NvlinkTopologyInventory, PcieDeviceInventory, ProcessorInventory,
    SystemInventory,
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
                kind: target.kind,
                name: target.name,
                compute_zone: target.compute_zone,
                node_group: target.node_group,
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
                        resource: error.resource.map(SanitizedUriReference::new),
                        message: SanitizedErrorMessage::new(error.message),
                    })
                    .collect(),
                target_error: None,
            }
        }
        Err(error) => NodeInventory {
            schema_version: INVENTORY_SCHEMA_VERSION.to_string(),
            collected_at,
            enrolled_at: None,
            kind: target.kind,
            name: target.name,
            compute_zone: target.compute_zone,
            node_group: target.node_group,
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
            target_error: Some(SanitizedErrorMessage::new(error)),
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
        name: manager.name,
        description: manager.description,
        id: manager.id,
        odata_id: sanitize_optional_uri(manager.odata_id),
        uuid: manager.uuid,
        manager_type: manager.manager_type,
        model: manager.model,
        firmware_version: manager.firmware_version,
        status: manager.status.map(map_status),
        links: manager.links.into_iter().map(map_link).collect(),
    }
}

fn map_system(system: SystemData, uuid: Option<InventoryUuid>) -> SystemInventory {
    SystemInventory {
        name: system.name,
        description: system.description,
        id: system.id,
        odata_id: sanitize_optional_uri(system.odata_id),
        uuid,
        manufacturer: system.manufacturer,
        model: system.model,
        sku: system.sku,
        serial_number: system.serial_number,
        bios_version: system.bios_version,
        host_name: system.host_name,
        asset_tag: system.asset_tag,
        power_state: system.power_state,
        status: system.status.map(map_status),
        processor_summary: system.processor_summary.map(map_processor_summary),
        memory_summary: system.memory_summary.map(map_memory_summary),
        secure_boot_enabled: system.secure_boot_enabled,
        processors: system.processors.into_iter().map(map_processor).collect(),
        links: system.links.into_iter().map(map_link).collect(),
    }
}

fn map_processor(processor: crate::source::ProcessorData) -> ProcessorInventory {
    ProcessorInventory {
        name: processor.name,
        description: processor.description,
        id: processor.id,
        odata_id: sanitize_optional_uri(processor.odata_id),
        uuid: processor.uuid,
        socket: processor.socket,
        processor_type: processor.processor_type,
        processor_architecture: processor.processor_architecture,
        instruction_set: processor.instruction_set,
        manufacturer: processor.manufacturer,
        model: processor.model,
        serial_number: processor.serial_number,
        part_number: processor.part_number,
        location: processor.location.map(map_location),
        nvidia_gpu: processor.nvidia_gpu.map(map_nvidia_gpu),
        max_speed_mhz: processor.max_speed_mhz,
        total_cores: processor.total_cores,
        total_threads: processor.total_threads,
        status: processor.status.map(map_status),
        links: processor.links.into_iter().map(map_link).collect(),
    }
}

fn map_chassis(chassis: ChassisData) -> ChassisInventory {
    ChassisInventory {
        name: chassis.name,
        description: chassis.description,
        id: chassis.id,
        odata_id: sanitize_optional_uri(chassis.odata_id),
        chassis_type: chassis.chassis_type,
        manufacturer: chassis.manufacturer,
        model: chassis.model,
        sku: chassis.sku,
        serial_number: chassis.serial_number,
        part_number: chassis.part_number,
        asset_tag: chassis.asset_tag,
        power_state: chassis.power_state,
        status: chassis.status.map(map_status),
        location: chassis.location.map(map_location),
        pcie_devices: chassis
            .pcie_devices
            .into_iter()
            .map(map_pcie_device)
            .collect(),
        links: chassis.links.into_iter().map(map_link).collect(),
    }
}

fn map_location(loc: crate::source::LocationData) -> InventoryLocation {
    InventoryLocation {
        part_location_context: loc.part_location_context,
        part_location: loc.part_location.map(|part| crate::models::PartLocation {
            service_label: part.service_label,
            location_type: part.location_type,
            location_ordinal_value: part.location_ordinal_value,
            reference: part.reference,
            orientation: part.orientation,
        }),
        placement: loc.placement.map(|placement| crate::models::Placement {
            rack: placement.rack,
            row: placement.row,
            room: placement.room,
            rack_offset: placement.rack_offset,
            rack_offset_units: placement.rack_offset_units,
            additional_info: placement.additional_info,
            facility_name: placement.facility_name,
        }),
    }
}

fn map_nvidia_gpu(gpu: crate::source::NvidiaGpuData) -> NvidiaGpuInventory {
    NvidiaGpuInventory {
        mig_mode_enabled: gpu.mig_mode_enabled,
        default_boost_clock_speed_mhz: gpu.default_boost_clock_speed_mhz,
        max_boost_clock_speed_mhz: gpu.max_boost_clock_speed_mhz,
        cc_mode_enabled: gpu.cc_mode_enabled,
        cc_dev_mode_enabled: gpu.cc_dev_mode_enabled,
        protected_pcie_mode_enabled: gpu.protected_pcie_mode_enabled,
        egm_mode_enabled: gpu.egm_mode_enabled,
        total_nv_links: gpu.total_nv_links,
        nvlink_topology: gpu.nvlink_topology.map(map_nvlink_topology),
    }
}

fn map_nvlink_topology(topology: crate::source::NvlinkTopologyData) -> NvlinkTopologyInventory {
    NvlinkTopologyInventory {
        tray_serial_number: topology.tray_serial_number,
        tray_slot_number: topology.tray_slot_number,
        tray_slot_index: topology.tray_slot_index,
        chassis_serial_number: topology.chassis_serial_number,
        host_id: topology.host_id,
        module_id: topology.module_id,
        system_guid: topology.system_guid,
        ib_guid: topology.ib_guid,
        lsb_ib_guid: topology.lsb_ib_guid,
        peer_type: topology.peer_type,
    }
}

fn map_pcie_device(device: crate::source::PcieDeviceData) -> PcieDeviceInventory {
    PcieDeviceInventory {
        name: device.name,
        description: device.description,
        id: device.id,
        odata_id: sanitize_optional_uri(device.odata_id),
        uuid: device.uuid,
        device_type: device.device_type,
        manufacturer: device.manufacturer,
        model: device.model,
        sku: device.sku,
        serial_number: device.serial_number,
        part_number: device.part_number,
        firmware_version: device.firmware_version,
        status: device.status.map(map_status),
        links: device.links.into_iter().map(map_link).collect(),
    }
}

fn map_firmware(firmware: crate::source::FirmwareData) -> FirmwareInventory {
    FirmwareInventory {
        description: firmware.description,
        related_item: firmware.related_item.map(|items| {
            items
                .into_iter()
                .map(|odata_id| InventoryLinkReference {
                    odata_id: SanitizedUriReference::new(odata_id),
                })
                .collect()
        }),
        service_id: firmware.service_id,
        id: firmware.id,
        odata_id: sanitize_optional_uri(firmware.odata_id),
        name: firmware.name,
        version: firmware.version,
        release_date: firmware.release_date,
        status: firmware.status.map(map_status),
        links: firmware.links.into_iter().map(map_link).collect(),
    }
}

fn map_processor_summary(
    summary: crate::source::ProcessorSummaryData,
) -> crate::models::ProcessorSummary {
    crate::models::ProcessorSummary {
        count: summary.count,
        core_count: summary.core_count,
        model: summary.model,
        logical_processor_count: summary.logical_processor_count,
        threading_enabled: summary.threading_enabled,
        status: summary.status.map(map_status),
    }
}

fn map_memory_summary(summary: crate::source::MemorySummaryData) -> crate::models::MemorySummary {
    crate::models::MemorySummary {
        total_system_memory_gib: summary.total_system_memory_gib,
        total_system_persistent_memory_gib: summary.total_system_persistent_memory_gib,
        memory_mirroring: summary.memory_mirroring,
        status: summary.status.map(map_status),
    }
}

fn map_status(status: crate::source::StatusData) -> InventoryStatus {
    InventoryStatus {
        conditions: status
            .conditions
            .map(|conditions| conditions.into_iter().map(map_condition).collect()),
        health: status.health,
        health_rollup: status.health_rollup,
        state: status.state,
    }
}

fn map_condition(condition: crate::source::ConditionData) -> InventoryCondition {
    InventoryCondition {
        timestamp: condition.timestamp,
        message_id: condition.message_id,
        message_args: condition
            .message_args
            .map(|args| args.into_iter().map(SanitizedErrorMessage::new).collect()),
        message: condition.message.map(SanitizedErrorMessage::new),
        severity: condition.severity,
        resolution: condition.resolution.map(SanitizedErrorMessage::new),
        condition_type: condition.condition_type,
        origin_of_condition: condition
            .origin_of_condition
            .map(|odata_id| InventoryReference {
                odata_id: SanitizedUriReference::new(odata_id),
            }),
    }
}

fn map_link((relation, link): (String, crate::source::LinkData)) -> (String, InventoryLink) {
    let value = link
        .into_iter()
        .map(|odata_id| InventoryLinkReference {
            odata_id: SanitizedUriReference::new(odata_id),
        })
        .collect();
    (relation, value)
}

fn sanitize_optional_uri(reference: Option<String>) -> Option<SanitizedUriReference> {
    reference.map(SanitizedUriReference::new)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::{Target, TargetKind};
    use crate::source::{
        ChassisData, FirmwareData, LinkData, ManagerData, ProcessorData, RedfishReport,
        ServiceRootData, SourceRecord, StatusData, SystemData,
    };

    #[test]
    fn maps_populated_status_conditions_and_origin_reference() {
        let mapped = map_status(StatusData {
            conditions: Some(vec![crate::source::ConditionData {
                timestamp: Some("2026-08-25T20:51:47Z".to_string()),
                message_id: "ResourceEvent.1.0.ResourceErrorsDetected".to_string(),
                message_args: Some(vec![
                    "CPU_1 Firmware".to_string(),
                    "EC_STRAP_MISMATCH".to_string(),
                ]),
                message: Some("CPU_1 Firmware detected EC_STRAP_MISMATCH.".to_string()),
                severity: Some("Critical".to_string()),
                resolution: Some("Reset the component and collect logs.".to_string()),
                condition_type: Some("Alert".to_string()),
                origin_of_condition: Some("/redfish/v1/Chassis/HGX_CPU_1".to_string()),
            }]),
            health: Some("Warning".to_string()),
            health_rollup: Some("Critical".to_string()),
            state: Some("Enabled".to_string()),
        });

        assert_eq!(mapped.health.as_deref(), Some("Warning"));
        assert_eq!(mapped.health_rollup.as_deref(), Some("Critical"));
        assert_eq!(mapped.state.as_deref(), Some("Enabled"));
        let conditions = mapped.conditions.expect("mapped conditions");
        assert_eq!(conditions.len(), 1);
        let condition = &conditions[0];
        assert_eq!(condition.timestamp.as_deref(), Some("2026-08-25T20:51:47Z"));
        assert_eq!(
            condition.message_id,
            "ResourceEvent.1.0.ResourceErrorsDetected"
        );
        assert_eq!(
            condition.message_args.as_deref(),
            Some(["CPU_1 Firmware".into(), "EC_STRAP_MISMATCH".into()].as_slice())
        );
        assert_eq!(
            condition.message.as_deref(),
            Some("CPU_1 Firmware detected EC_STRAP_MISMATCH.")
        );
        assert_eq!(condition.severity.as_deref(), Some("Critical"));
        assert_eq!(
            condition.resolution.as_deref(),
            Some("Reset the component and collect logs.")
        );
        assert_eq!(condition.condition_type.as_deref(), Some("Alert"));
        assert_eq!(
            condition
                .origin_of_condition
                .as_ref()
                .expect("origin reference")
                .odata_id,
            "/redfish/v1/Chassis/HGX_CPU_1"
        );
    }

    #[test]
    fn sanitizes_redfish_urls_before_building_inventory_models() {
        let inventory = map_report(RedfishReport {
            service_root: ServiceRootData::default(),
            managers: vec![ManagerData {
                name: "BMC".into(),
                description: None,
                id: "BMC".into(),
                odata_id: Some(
                    "//user:secret@bmc.example/redfish/v1/Managers/BMC?token=x#item".into(),
                ),
                uuid: None,
                manager_type: None,
                model: None,
                firmware_version: None,
                status: Some(StatusData {
                    conditions: Some(vec![crate::source::ConditionData {
                        timestamp: None,
                        message_id: "Alert.1.0.Test".into(),
                        message_args: Some(vec!["https://bmc.example/arg?token=x".into()]),
                        message: Some(
                            "request //user:secret@bmc.example/path?token=x failed".into(),
                        ),
                        severity: None,
                        resolution: Some("retry https://bmc.example/retry?token=x".into()),
                        condition_type: None,
                        origin_of_condition: Some(
                            "//bmc.example/redfish/v1/Managers/BMC?token=x#condition".into(),
                        ),
                    }]),
                    health: None,
                    health_rollup: None,
                    state: None,
                }),
                links: std::collections::BTreeMap::from([(
                    "ManagerForServers".into(),
                    vec!["//user:secret@bmc.example/redfish/v1/Systems/1?token=x#item".into()],
                )]),
            }],
            systems: Vec::new(),
            chassis: Vec::new(),
            firmware: Vec::new(),
            errors: vec![crate::source::InventoryErrorData {
                domain: "managers".into(),
                resource: Some("//bmc.example/redfish/v1/Managers/BMC?token=x#item".into()),
                message: "failed https://bmc.example/redfish/v1/Managers/BMC?token=x".into(),
            }],
        });

        let manager = &inventory.managers[0];
        assert_eq!(
            manager.odata_id.as_deref(),
            Some("/redfish/v1/Managers/BMC#item")
        );
        assert_link(
            &manager.links,
            "ManagerForServers",
            "/redfish/v1/Systems/1#item",
        );
        let condition = &manager
            .status
            .as_ref()
            .unwrap()
            .conditions
            .as_ref()
            .unwrap()[0];
        assert_eq!(
            condition.message_args.as_deref(),
            Some(["/arg".into()].as_slice())
        );
        assert_eq!(condition.message.as_deref(), Some("request /path failed"));
        assert_eq!(condition.resolution.as_deref(), Some("retry /retry"));
        assert_eq!(
            condition.origin_of_condition.as_ref().unwrap().odata_id,
            "/redfish/v1/Managers/BMC#condition"
        );
        assert_eq!(
            inventory.domain_errors[0].resource.as_deref(),
            Some("/redfish/v1/Managers/BMC#item")
        );
        assert_eq!(
            inventory.domain_errors[0].message,
            "failed /redfish/v1/Managers/BMC"
        );

        let rendered = serde_json::to_string(&inventory).unwrap();
        assert!(!rendered.contains("user:secret@"));
        assert!(!rendered.contains("token=x"));

        let failed = map(SourceRecord {
            target: target(),
            collected_at: collected_at(),
            result: Err("failed //user:secret@bmc.example/redfish/v1?token=x".into()),
        });
        assert_eq!(failed.target_error.as_deref(), Some("failed /redfish/v1"));
    }

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
    fn maps_configured_machine_name_for_success_and_failure() {
        let mut successful_target = target();
        successful_target.name = Some("node-01".to_string());
        successful_target.compute_zone = Some("us-west-dc1".to_string());
        successful_target.node_group = Some("gb200-training".to_string());
        let successful = map_record(
            SourceRecord {
                target: successful_target,
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
            None,
        );

        let mut failed_target = target();
        failed_target.name = Some("node-02".to_string());
        let failed = map_record(
            SourceRecord {
                target: failed_target,
                collected_at: collected_at(),
                result: Err("unreachable".to_string()),
            },
            None,
        );

        assert_eq!(successful.name.as_deref(), Some("node-01"));
        assert_eq!(successful.compute_zone.as_deref(), Some("us-west-dc1"));
        assert_eq!(successful.node_group.as_deref(), Some("gb200-training"));
        assert_eq!(failed.name.as_deref(), Some("node-02"));
    }

    #[test]
    fn maps_summary_health_into_nested_payload() {
        let summary = crate::source::ProcessorSummaryData {
            count: Some(2),
            status: Some(crate::source::StatusData {
                health: Some("Critical".into()),
                conditions: Some(vec![]),
                health_rollup: None,
                state: None,
            }),
            ..Default::default()
        };
        let value = serde_json::to_value(map_processor_summary(summary)).unwrap();
        assert_eq!(value["count"], 2);
        assert_eq!(value["status"]["health"], "Critical");
        assert_eq!(value["status"]["conditions"], serde_json::json!([]));
        let memory = crate::source::MemorySummaryData {
            total_system_memory_gib: Some(480.0),
            status: Some(crate::source::StatusData {
                health: Some("Warning".into()),
                conditions: None,
                health_rollup: None,
                state: None,
            }),
            ..Default::default()
        };
        let value = serde_json::to_value(map_memory_summary(memory)).unwrap();
        assert_eq!(value["totalSystemMemoryGiB"], 480.0);
        assert_eq!(value["status"]["health"], "Warning");
    }

    #[test]
    fn maps_all_links_as_arrays() {
        let (relation, single) = map_link((
            "ContainedBy".into(),
            vec!["/redfish/v1/Chassis/Rack".into()],
        ));
        assert_eq!(relation, "ContainedBy");
        assert_eq!(
            serde_json::to_value(single).unwrap(),
            serde_json::json!([{"odataId": "/redfish/v1/Chassis/Rack"}])
        );
        let (_, empty) = map_link(("Contains".into(), vec![]));
        assert_eq!(serde_json::to_value(empty).unwrap(), serde_json::json!([]));
    }

    #[test]
    fn prefers_system_with_bios() {
        let mut mapped_processor = processor("CPU1");
        mapped_processor.links = std::collections::BTreeMap::from([link(
            "PCIeDevice",
            "/redfish/v1/Chassis/HGX_GPU_0/PCIeDevices/GPU_0",
        )]);
        let mut mapped_system = system("System")
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
            .with_processor(mapped_processor);
        mapped_system.links =
            std::collections::BTreeMap::from([link("ManagedBy", "/redfish/v1/Managers/BMC_0")]);

        let inventory = map(SourceRecord {
            target: target(),
            collected_at: collected_at(),
            result: Ok(RedfishReport {
                service_root: ServiceRootData::default(),
                managers: Vec::new(),
                systems: vec![system("BMC"), mapped_system],
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
        assert_eq!(
            system
                .status
                .as_ref()
                .and_then(|status| status.state.as_deref()),
            Some("Enabled")
        );
        assert_eq!(
            system.uuid.as_ref().expect("uuid").to_string(),
            "9aee71e4-6e80-11ee-a8ac-74d4dd550187"
        );
        assert_eq!(system.processors.len(), 1);
        assert_eq!(system.processors[0].id, "CPU1");
        assert_eq!(
            system.processors[0].uuid.as_deref(),
            Some("22222222-3333-4444-5555-666666666666")
        );
        assert_eq!(system.processors[0].processor_type.as_deref(), Some("cpu"));
        assert_eq!(
            system.processors[0].serial_number.as_deref(),
            Some("CPU-SN123")
        );
        assert_eq!(system.processors[0].total_cores, Some(72));
        assert_link(&system.links, "ManagedBy", "/redfish/v1/Managers/BMC_0");
        assert_link(
            &system.processors[0].links,
            "PCIeDevice",
            "/redfish/v1/Chassis/HGX_GPU_0/PCIeDevices/GPU_0",
        );
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
                        name: "Chassis".into(),
                        description: None,
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
                        status: None,
                        location: None,
                        pcie_devices: vec![pcie_device("GPU_0")],
                        links: std::collections::BTreeMap::from([link(
                            "Processors",
                            "/redfish/v1/Systems/System/Processors/GPU_0",
                        )]),
                    },
                    ChassisData {
                        name: "Chassis".into(),
                        description: None,
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
                        status: None,
                        location: None,
                        pcie_devices: Vec::new(),
                        links: Default::default(),
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
        assert_link(
            &inventory.chassis[0].links,
            "Processors",
            "/redfish/v1/Systems/System/Processors/GPU_0",
        );
        assert_link(
            &inventory.chassis[0].pcie_devices[0].links,
            "Chassis",
            "/redfish/v1/Chassis/BMC_0",
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
                name: "Manager".into(),
                description: Some("  Controller  ".into()),
                id: "BMC_0".to_string(),
                odata_id: Some("/redfish/v1/Managers/BMC_0".to_string()),
                uuid: Some("manager-uuid".to_string()),
                manager_type: Some("bmc".to_string()),
                model: Some("OpenBMC".to_string()),
                firmware_version: Some("3.19.00".to_string()),
                status: Some(status("Enabled", "OK", "OK")),
                links: std::collections::BTreeMap::from([link(
                    "ManagerForServers",
                    "/redfish/v1/Systems/System",
                )]),
            }],
            systems: Vec::new(),
            chassis: Vec::new(),
            firmware: vec![FirmwareData {
                description: None,
                related_item: None,
                service_id: "FirmwareInventory".to_string(),
                id: "BMCImage1".to_string(),
                odata_id: Some("/redfish/v1/UpdateService/FirmwareInventory/BMCImage1".to_string()),
                name: "BMCImage1".to_string(),
                version: Some("3.19.00".to_string()),
                release_date: None,
                status: Some(status("StandbyOffline", "Critical", "Warning")),
                links: std::collections::BTreeMap::from([link(
                    "ActiveTargets",
                    "/redfish/v1/Managers/BMC_0",
                )]),
            }],
            errors: Vec::new(),
        });

        assert_eq!(inventory.source.redfish_version.as_deref(), Some("1.15.1"));
        assert_eq!(inventory.source.vendor.as_deref(), Some("NVIDIA BMC"));
        assert!(inventory.uuid.is_some());
        assert_eq!(inventory.managers.len(), 1);
        let manager = inventory.managers.first().expect("manager");
        assert_eq!(manager.id, "BMC_0");
        assert_eq!(manager.description.as_deref(), Some("  Controller  "));
        assert_eq!(
            manager.odata_id.as_deref(),
            Some("/redfish/v1/Managers/BMC_0")
        );
        assert_eq!(manager.manager_type.as_deref(), Some("bmc"));
        let manager_status = manager.status.as_ref().expect("manager status");
        assert_eq!(manager_status.state.as_deref(), Some("Enabled"));
        assert_eq!(manager_status.health.as_deref(), Some("OK"));
        assert_eq!(manager_status.health_rollup.as_deref(), Some("OK"));
        assert_eq!(manager.links.len(), 1);
        assert_link(
            &manager.links,
            "ManagerForServers",
            "/redfish/v1/Systems/System",
        );
        assert!(inventory.systems.is_empty());
        assert_eq!(inventory.firmware.len(), 1);
        assert_eq!(
            inventory.firmware[0].odata_id.as_deref(),
            Some("/redfish/v1/UpdateService/FirmwareInventory/BMCImage1")
        );
        let firmware_status = inventory.firmware[0]
            .status
            .as_ref()
            .expect("firmware status");
        assert_eq!(firmware_status.state.as_deref(), Some("StandbyOffline"));
        assert_eq!(firmware_status.health.as_deref(), Some("Critical"));
        assert_eq!(firmware_status.health_rollup.as_deref(), Some("Warning"));
        assert_link(
            &inventory.firmware[0].links,
            "ActiveTargets",
            "/redfish/v1/Managers/BMC_0",
        );
    }

    fn link(relation: &str, odata_id: &str) -> (String, LinkData) {
        (relation.to_string(), vec![odata_id.to_string()])
    }

    fn assert_link(
        links: &std::collections::BTreeMap<String, InventoryLink>,
        relation: &str,
        odata_id: &str,
    ) {
        assert_eq!(links.len(), 1);
        assert_eq!(
            serde_json::to_value(&links[relation]).unwrap(),
            serde_json::json!([{"odataId": odata_id}])
        );
    }

    fn pcie_device(id: &str) -> crate::source::PcieDeviceData {
        crate::source::PcieDeviceData {
            name: id.to_string(),
            description: None,
            id: id.to_string(),
            odata_id: None,
            uuid: None,
            device_type: None,
            manufacturer: None,
            model: None,
            sku: None,
            serial_number: None,
            part_number: None,
            firmware_version: None,
            status: None,
            links: std::collections::BTreeMap::from([link("Chassis", "/redfish/v1/Chassis/BMC_0")]),
        }
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
            kind: TargetKind::Compute,
            name: None,
            compute_zone: None,
            node_group: None,
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
            name: "System".into(),
            description: None,
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
            status: None,
            processor_summary: None,
            memory_summary: None,
            secure_boot_enabled: None,
            processors: Vec::new(),
            links: Default::default(),
        }
    }

    fn processor(id: &str) -> ProcessorData {
        ProcessorData {
            name: "Processor".into(),
            description: None,
            id: id.to_string(),
            odata_id: Some(format!("/redfish/v1/Systems/System/Processors/{id}")),
            uuid: Some("22222222-3333-4444-5555-666666666666".to_string()),
            socket: Some("CPU Socket 1".to_string()),
            processor_type: Some("cpu".to_string()),
            processor_architecture: Some("ARM".to_string()),
            instruction_set: Some("ARM-A64".to_string()),
            manufacturer: Some("NVIDIA".to_string()),
            model: Some("Grace".to_string()),
            serial_number: Some("CPU-SN123".to_string()),
            part_number: Some("CPU-PN123".to_string()),
            location: None,
            nvidia_gpu: None,
            max_speed_mhz: Some(3600),
            total_cores: Some(72),
            total_threads: Some(72),
            status: Some(status("Enabled", "OK", "OK")),
            links: Default::default(),
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
            self.status.get_or_insert_with(empty_status).state = Some(status_state.to_string());
            self
        }

        fn with_health(mut self, health: &str) -> Self {
            self.status.get_or_insert_with(empty_status).health = Some(health.to_string());
            self
        }

        fn with_cpu(mut self, count: i64, cores: i64) -> Self {
            self.processor_summary = Some(crate::source::ProcessorSummaryData {
                count: Some(count),
                core_count: Some(cores),
                ..Default::default()
            });
            self
        }

        fn with_memory_gib(mut self, memory_gib: f64) -> Self {
            self.memory_summary = Some(crate::source::MemorySummaryData {
                total_system_memory_gib: Some(memory_gib),
                ..Default::default()
            });
            self
        }

        fn with_processor(mut self, processor: ProcessorData) -> Self {
            self.processors.push(processor);
            self
        }
    }

    fn empty_status() -> StatusData {
        StatusData {
            conditions: None,
            health: None,
            health_rollup: None,
            state: None,
        }
    }

    fn status(state: &str, health: &str, health_rollup: &str) -> StatusData {
        StatusData {
            conditions: Some(Vec::new()),
            health: Some(health.to_string()),
            health_rollup: Some(health_rollup.to_string()),
            state: Some(state.to_string()),
        }
    }
}
