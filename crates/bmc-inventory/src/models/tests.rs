use serde_json::json;

use super::*;

#[test]
fn inventory_condition_serializes_diagnostic_fields() {
    let condition = InventoryCondition {
        timestamp: Some("2026-08-25T20:51:47Z".to_string()),
        message_id: "ResourceEvent.1.0.ResourceErrorsDetected".to_string(),
        message_args: Some(vec![
            "CPU_1 Firmware".to_string(),
            "EC_STRAP_MISMATCH".to_string(),
        ]),
        message: Some("The resource property CPU_1 Firmware has detected errors.".to_string()),
        severity: Some("Critical".to_string()),
        resolution: Some("Reset the component and collect logs.".to_string()),
        condition_type: Some("Alert".to_string()),
        origin_of_condition: Some(InventoryReference {
            odata_id: "/redfish/v1/Chassis/HGX_CPU_1".to_string(),
        }),
    };

    assert_eq!(
        serde_json::to_value(condition).expect("condition json"),
        json!({
            "timestamp": "2026-08-25T20:51:47Z",
            "messageId": "ResourceEvent.1.0.ResourceErrorsDetected",
            "messageArgs": ["CPU_1 Firmware", "EC_STRAP_MISMATCH"],
            "message": "The resource property CPU_1 Firmware has detected errors.",
            "severity": "Critical",
            "resolution": "Reset the component and collect logs.",
            "conditionType": "Alert",
            "originOfCondition": {
                "@odata.id": "/redfish/v1/Chassis/HGX_CPU_1"
            }
        })
    );
}

fn status(state: &str, health: &str, health_rollup: &str) -> InventoryStatus {
    InventoryStatus {
        conditions: Some(Vec::new()),
        health: Some(health.to_string()),
        health_rollup: Some(health_rollup.to_string()),
        state: Some(state.to_string()),
    }
}

#[test]
fn node_inventory_serializes_as_backend_payload() {
    let inventory = NodeInventory {
        schema_version: INVENTORY_SCHEMA_VERSION.to_string(),
        collected_at: "2026-01-01T00:00:00Z".to_string(),
        enrolled_at: Some("2026-01-02T00:00:00Z".to_string()),
        kind: TargetKind::NvSwitch,
        name: Some("node-01".to_string()),
        compute_zone: Some("us-west-dc1".to_string()),
        node_group: Some("gb200-training".to_string()),
        uuid: InventoryUuid::parse("9AEE71E4-6E80-11EE-A8AC-74D4DD550187"),
        primary_system_id: Some("System".to_string()),
        systems: vec![SystemInventory {
            name: "System".into(),
            description: None,
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
            status: Some(status("Enabled", "OK", "Warning")),
            processor_summary: Some(crate::models::ProcessorSummary {
                count: Some(1),
                core_count: Some(72),
                model: Some("Grace".into()),
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
                odata_id: Some("/redfish/v1/Systems/System/Processors/CPU1".to_string()),
                uuid: Some("22222222-3333-4444-5555-666666666666".to_string()),
                socket: Some("CPU Socket 1".to_string()),
                processor_type: Some("cpu".to_string()),
                processor_architecture: Some("ARM".to_string()),
                instruction_set: Some("ARM-A64".to_string()),
                manufacturer: Some("NVIDIA".to_string()),
                model: Some("Grace".to_string()),
                serial_number: Some("CPU-SN123".to_string()),
                part_number: Some("CPU-PN123".to_string()),
                location: Some(ChassisLocation {
                    part_location: Some(PartLocation {
                        service_label: Some("GPU_SXM_1".to_string()),
                        location_type: Some("Slot".to_string()),
                        location_ordinal_value: Some(1),
                        reference: None,
                        orientation: Some("LeftToRight".to_string()),
                    }),
                    placement: None,
                    part_location_context: Some("HGX_Chassis_0/ProcessorModule_0".to_string()),
                }),
                nvidia_gpu: Some(NvidiaGpuInventory {
                    mig_mode_enabled: Some(false),
                    default_boost_clock_speed_mhz: Some(1200),
                    max_boost_clock_speed_mhz: Some(1800),
                    cc_mode_enabled: Some(true),
                    cc_dev_mode_enabled: None,
                    protected_pcie_mode_enabled: None,
                    egm_mode_enabled: None,
                    total_nv_links: Some(18),
                    nvlink_topology: Some(NvlinkTopologyInventory {
                        tray_serial_number: None,
                        tray_slot_number: None,
                        tray_slot_index: Some(2),
                        chassis_serial_number: None,
                        host_id: None,
                        module_id: None,
                        system_guid: Some("system-guid".to_string()),
                        ib_guid: None,
                        lsb_ib_guid: None,
                        peer_type: None,
                    }),
                }),
                max_speed_mhz: Some(3600),
                total_cores: Some(72),
                total_threads: Some(72),
                status: Some(status("Enabled", "OK", "OK")),
                links: Default::default(),
            }],
            links: Default::default(),
        }],
        source: InventorySourceInfo {
            source_type: "redfish".to_string(),
            mac: "00:11:22:33:44:55".to_string(),
            address: "10.0.0.1".to_string(),
            host_name: Some("bmc-a.example.com".to_string()),
            redfish_version: Some("1.15.1".to_string()),
            service_uuid: Some("870155dd-d474-aca8-11ee-6e809af87c0c".to_string()),
            vendor: Some("NVIDIA".to_string()),
        },
        managers: vec![ManagerInventory {
            name: "Manager".into(),
            description: None,
            id: "BMC_0".to_string(),
            odata_id: Some("/redfish/v1/Managers/BMC_0".to_string()),
            uuid: Some("11111111-2222-3333-4444-555555555555".to_string()),
            manager_type: Some("bmc".to_string()),
            model: Some("OpenBMC".to_string()),
            firmware_version: Some("3.19.00".to_string()),
            status: Some(status("Enabled", "OK", "OK")),
            links: std::collections::BTreeMap::from([(
                "ManagerForServers".into(),
                InventoryLink::Multiple(vec![InventoryLinkReference {
                    odata_id: "/redfish/v1/Systems/System".into(),
                }]),
            )]),
        }],
        chassis: vec![ChassisInventory {
            name: "Chassis".into(),
            description: None,
            id: "BMC_0".to_string(),
            odata_id: Some("/redfish/v1/Chassis/BMC_0".to_string()),
            chassis_type: Some("module".to_string()),
            manufacturer: Some("NVIDIA".to_string()),
            model: Some("BMC".to_string()),
            sku: Some("CH-SKU".to_string()),
            serial_number: Some("CH123".to_string()),
            part_number: Some("PN123".to_string()),
            asset_tag: Some("rack-12".to_string()),
            power_state: Some("on".to_string()),
            status: Some(status("Enabled", "OK", "OK")),
            location: Some(ChassisLocation {
                part_location: Some(crate::models::PartLocation {
                    service_label: Some("Bay 1".to_string()),
                    ..Default::default()
                }),
                placement: Some(crate::models::Placement {
                    rack: Some("R12".to_string()),
                    row: Some("A".to_string()),
                    room: None,
                    rack_offset: Some(7),
                    ..Default::default()
                }),
                part_location_context: None,
            }),
            pcie_devices: vec![PcieDeviceInventory {
                name: "PcieDevice".into(),
                description: None,
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
                status: Some(status("Enabled", "OK", "OK")),
                links: Default::default(),
            }],
            links: Default::default(),
        }],
        firmware: vec![FirmwareInventory {
            description: None,
            related_item: None,
            service_id: "FirmwareInventory".to_string(),
            id: "BMC".to_string(),
            odata_id: Some("/redfish/v1/UpdateService/FirmwareInventory/BMC".to_string()),
            name: "BMC".to_string(),
            version: Some("3.19.00".to_string()),
            release_date: Some("2026-01-01T00:00:00Z".to_string()),
            status: Some(status("StandbyOffline", "Critical", "Warning")),
            links: Default::default(),
        }],
        domain_errors: vec![DomainError {
            domain: "chassis.pcieDevices".to_string(),
            resource: Some("HGX_GPU_SXM_0".to_string()),
            message: "timeout".to_string(),
        }],
        target_error: None,
    };

    let value = serde_json::to_value(inventory).expect("json");

    assert_eq!(
        value,
        json!({
            "schemaVersion": "inventory.v1alpha1",
            "collectedAt": "2026-01-01T00:00:00Z",
            "enrolledAt": "2026-01-02T00:00:00Z",
            "kind": "nvswitch",
            "name": "node-01",
            "computeZone": "us-west-dc1",
            "nodeGroup": "gb200-training",
            "uuid": "9aee71e4-6e80-11ee-a8ac-74d4dd550187",
            "primarySystemId": "System",
            "systems": [{
                "name": "System",
                "description": null,
                "id": "System",
                "@odata.id": "/redfish/v1/Systems/System",
                "uuid": "9aee71e4-6e80-11ee-a8ac-74d4dd550187",
                "manufacturer": "NVIDIA",
                "model": "GB200",
                "sku": "SKU-GB200",
                "serialNumber": "SN123",
                "biosVersion": "3A21",
                "hostName": "node-a.example.com",
                "assetTag": "rack-12-tray-3",
                "powerState": "on",
                "status": {
                    "conditions": [],
                    "health": "OK",
                    "healthRollup": "Warning",
                    "state": "Enabled"
                },
                "processorSummary": {"count": 1, "coreCount": 72, "model": "Grace", "logicalProcessorCount": null, "threadingEnabled": null, "status": null},
                "memorySummary": {"totalSystemMemoryGiB": 480.0, "totalSystemPersistentMemoryGiB": null, "memoryMirroring": null, "status": null},
                "secureBootEnabled": true,
                "processors": [{
                "name": "Processor",
                "description": null,
                    "id": "CPU1",
                    "@odata.id": "/redfish/v1/Systems/System/Processors/CPU1",
                    "uuid": "22222222-3333-4444-5555-666666666666",
                    "socket": "CPU Socket 1",
                    "processorType": "cpu",
                    "processorArchitecture": "ARM",
                    "instructionSet": "ARM-A64",
                    "manufacturer": "NVIDIA",
                    "model": "Grace",
                    "serialNumber": "CPU-SN123",
                    "partNumber": "CPU-PN123",
                    "location": {
                        "partLocation": {
                            "serviceLabel": "GPU_SXM_1",
                            "locationType": "Slot",
                            "locationOrdinalValue": 1,
                            "reference": null,
                            "orientation": "LeftToRight"
                        },
                        "placement": null,
                        "partLocationContext": "HGX_Chassis_0/ProcessorModule_0"
                    },
                    "nvidiaGpu": {
                        "migModeEnabled": false,
                        "defaultBoostClockSpeedMhz": 1200,
                        "maxBoostClockSpeedMhz": 1800,
                        "ccModeEnabled": true,
                        "ccDevModeEnabled": null,
                        "protectedPcieModeEnabled": null,
                        "egmModeEnabled": null,
                        "totalNvLinks": 18,
                        "nvlinkTopology": {
                            "traySerialNumber": null,
                            "traySlotNumber": null,
                            "traySlotIndex": 2,
                            "chassisSerialNumber": null,
                            "hostId": null,
                            "moduleId": null,
                            "systemGuid": "system-guid",
                            "ibGuid": null,
                            "lsbIbGuid": null,
                            "peerType": null
                        }
                    },
                    "maxSpeedMhz": 3600,
                    "totalCores": 72,
                    "totalThreads": 72,
                    "status": {
                        "conditions": [],
                        "health": "OK",
                        "healthRollup": "OK",
                        "state": "Enabled"
                    },
                    "links": {}
                }],
                "links": {}
            }],
            "source": {
                "sourceType": "redfish",
                "mac": "00:11:22:33:44:55",
                "address": "10.0.0.1",
                "hostName": "bmc-a.example.com",
                "redfishVersion": "1.15.1",
                "serviceUuid": "870155dd-d474-aca8-11ee-6e809af87c0c",
                "vendor": "NVIDIA"
            },
            "managers": [{
                "name": "Manager",
                "description": null,
                "id": "BMC_0",
                "@odata.id": "/redfish/v1/Managers/BMC_0",
                "uuid": "11111111-2222-3333-4444-555555555555",
                "managerType": "bmc",
                "model": "OpenBMC",
                "firmwareVersion": "3.19.00",
                "status": {
                    "conditions": [],
                    "health": "OK",
                    "healthRollup": "OK",
                    "state": "Enabled"
                },
                "links": {"ManagerForServers": [{"@odata.id": "/redfish/v1/Systems/System"}]}
            }],
            "chassis": [{
                "name": "Chassis",
                "description": null,
                "id": "BMC_0",
                "@odata.id": "/redfish/v1/Chassis/BMC_0",
                "chassisType": "module",
                "manufacturer": "NVIDIA",
                "model": "BMC",
                "sku": "CH-SKU",
                "serialNumber": "CH123",
                "partNumber": "PN123",
                "assetTag": "rack-12",
                "powerState": "on",
                "status": {
                    "conditions": [],
                    "health": "OK",
                    "healthRollup": "OK",
                    "state": "Enabled"
                },
                "location": {
                    "partLocation": {"serviceLabel": "Bay 1", "locationType": null, "locationOrdinalValue": null, "reference": null, "orientation": null},
                    "placement": {"rack": "R12", "row": "A", "room": null, "rackOffset": 7, "rackOffsetUnits": null, "additionalInfo": null, "facilityName": null},
                    "partLocationContext": null
                },
                "pcieDevices": [{
                "name": "PcieDevice",
                "description": null,
                    "id": "GPU1",
                    "@odata.id": "/redfish/v1/Chassis/BMC_0/PCIeDevices/GPU1",
                    "uuid": null,
                    "deviceType": "single_function",
                    "manufacturer": "NVIDIA",
                    "model": "B200",
                    "sku": "B200-SXM",
                    "serialNumber": "GPU-SN456",
                    "partNumber": "GPU-PN456",
                    "firmwareVersion": "96.00.5E.00.02",
                    "status": {
                        "conditions": [],
                        "health": "OK",
                        "healthRollup": "OK",
                        "state": "Enabled"
                    },
                    "links": {}
                }],
                "links": {}
            }],
            "firmware": [{

                "description": null,
                "relatedItem": null,
                "serviceId": "FirmwareInventory",
                "id": "BMC",
                "@odata.id": "/redfish/v1/UpdateService/FirmwareInventory/BMC",
                "name": "BMC",
                "version": "3.19.00",
                "releaseDate": "2026-01-01T00:00:00Z",
                "status": {
                    "conditions": [],
                    "health": "Critical",
                    "healthRollup": "Warning",
                    "state": "StandbyOffline"
                },
                "links": {}
            }],
            "domainErrors": [{
                "domain": "chassis.pcieDevices",
                "resource": "HGX_GPU_SXM_0",
                "message": "timeout"
            }],
            "targetError": null
        })
    );
}

#[test]
fn target_error_inventory_serializes_absent_uuid() {
    let inventory = NodeInventory {
        schema_version: INVENTORY_SCHEMA_VERSION.to_string(),
        collected_at: "2026-01-01T00:00:00Z".to_string(),
        enrolled_at: None,
        kind: TargetKind::Unknown,
        name: None,
        compute_zone: None,
        node_group: None,
        uuid: None,
        source: InventorySourceInfo {
            source_type: "redfish".to_string(),
            mac: "00:11:22:33:44:55".to_string(),
            address: "10.0.0.1".to_string(),
            host_name: None,
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
        target_error: Some("malformed system UUID".to_string()),
    };

    let value = serde_json::to_value(inventory).expect("json");

    assert!(value["uuid"].is_null());
    assert!(value.get("name").is_none());
    assert!(value.get("computeZone").is_none());
    assert!(value.get("nodeGroup").is_none());
    assert!(value["enrolledAt"].is_null());
    assert_eq!(value["targetError"], "malformed system UUID");
}
