use serde_json::json;

use super::*;

#[test]
fn node_inventory_serializes_as_backend_payload() {
    let inventory = NodeInventory {
        schema_version: INVENTORY_SCHEMA_VERSION.to_string(),
        collected_at: "2026-01-01T00:00:00Z".to_string(),
        enrolled_at: Some("2026-01-02T00:00:00Z".to_string()),
        name: Some("node-01".to_string()),
        compute_zone: Some("us-west-dc1".to_string()),
        node_group: Some("gb200-training".to_string()),
        uuid: InventoryUuid::parse("9AEE71E4-6E80-11EE-A8AC-74D4DD550187"),
        primary_system_id: Some("System".to_string()),
        systems: vec![SystemInventory {
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
            cpu_model: Some("Grace".to_string()),
            memory_gib: Some(480.0),
            secure_boot_enabled: Some(true),
            processors: vec![ProcessorInventory {
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
                max_speed_mhz: Some(3600),
                total_cores: Some(72),
                total_threads: Some(72),
                status_state: Some("Enabled".to_string()),
                health: Some("OK".to_string()),
                health_rollup: Some("OK".to_string()),
            }],
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
            id: "BMC_0".to_string(),
            odata_id: Some("/redfish/v1/Managers/BMC_0".to_string()),
            uuid: Some("11111111-2222-3333-4444-555555555555".to_string()),
            manager_type: Some("bmc".to_string()),
            model: Some("OpenBMC".to_string()),
            firmware_version: Some("3.19.00".to_string()),
            status_state: Some("Enabled".to_string()),
            health: Some("OK".to_string()),
            health_rollup: Some("OK".to_string()),
        }],
        chassis: vec![ChassisInventory {
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
            status_state: Some("Enabled".to_string()),
            health: Some("OK".to_string()),
            health_rollup: Some("OK".to_string()),
            location: Some(ChassisLocation {
                service_label: Some("Bay 1".to_string()),
                rack: Some("R12".to_string()),
                row: Some("A".to_string()),
                room: None,
                rack_offset: Some(7),
            }),
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
            status_state: Some("StandbyOffline".to_string()),
            health: Some("Critical".to_string()),
            health_rollup: Some("Warning".to_string()),
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
            "name": "node-01",
            "computeZone": "us-west-dc1",
            "nodeGroup": "gb200-training",
            "uuid": "9aee71e4-6e80-11ee-a8ac-74d4dd550187",
            "primarySystemId": "System",
            "systems": [{
                "id": "System",
                "odataId": "/redfish/v1/Systems/System",
                "uuid": "9aee71e4-6e80-11ee-a8ac-74d4dd550187",
                "manufacturer": "NVIDIA",
                "model": "GB200",
                "sku": "SKU-GB200",
                "serialNumber": "SN123",
                "biosVersion": "3A21",
                "hostName": "node-a.example.com",
                "assetTag": "rack-12-tray-3",
                "powerState": "on",
                "statusState": "Enabled",
                "health": "OK",
                "healthRollup": "Warning",
                "cpuCount": 1,
                "cpuCoreCount": 72,
                "cpuModel": "Grace",
                "memoryGib": 480.0,
                "secureBootEnabled": true,
                "processors": [{
                    "id": "CPU1",
                    "odataId": "/redfish/v1/Systems/System/Processors/CPU1",
                    "uuid": "22222222-3333-4444-5555-666666666666",
                    "socket": "CPU Socket 1",
                    "processorType": "cpu",
                    "processorArchitecture": "ARM",
                    "instructionSet": "ARM-A64",
                    "manufacturer": "NVIDIA",
                    "model": "Grace",
                    "serialNumber": "CPU-SN123",
                    "maxSpeedMhz": 3600,
                    "totalCores": 72,
                    "totalThreads": 72,
                    "statusState": "Enabled",
                    "health": "OK",
                    "healthRollup": "OK"
                }]
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
                "id": "BMC_0",
                "odataId": "/redfish/v1/Managers/BMC_0",
                "uuid": "11111111-2222-3333-4444-555555555555",
                "managerType": "bmc",
                "model": "OpenBMC",
                "firmwareVersion": "3.19.00",
                "statusState": "Enabled",
                "health": "OK",
                "healthRollup": "OK"
            }],
            "chassis": [{
                "id": "BMC_0",
                "odataId": "/redfish/v1/Chassis/BMC_0",
                "chassisType": "module",
                "manufacturer": "NVIDIA",
                "model": "BMC",
                "sku": "CH-SKU",
                "serialNumber": "CH123",
                "partNumber": "PN123",
                "assetTag": "rack-12",
                "powerState": "on",
                "statusState": "Enabled",
                "health": "OK",
                "healthRollup": "OK",
                "location": {
                    "serviceLabel": "Bay 1",
                    "rack": "R12",
                    "row": "A",
                    "room": null,
                    "rackOffset": 7
                },
                "pcieDevices": [{
                    "id": "GPU1",
                    "odataId": "/redfish/v1/Chassis/BMC_0/PCIeDevices/GPU1",
                    "uuid": null,
                    "deviceType": "single_function",
                    "manufacturer": "NVIDIA",
                    "model": "B200",
                    "sku": "B200-SXM",
                    "serialNumber": "GPU-SN456",
                    "partNumber": "GPU-PN456",
                    "firmwareVersion": "96.00.5E.00.02",
                    "statusState": "Enabled",
                    "health": "OK",
                    "healthRollup": "OK"
                }]
            }],
            "firmware": [{
                "serviceId": "FirmwareInventory",
                "id": "BMC",
                "odataId": "/redfish/v1/UpdateService/FirmwareInventory/BMC",
                "name": "BMC",
                "version": "3.19.00",
                "releaseDate": "2026-01-01T00:00:00Z",
                "statusState": "StandbyOffline",
                "health": "Critical",
                "healthRollup": "Warning"
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
