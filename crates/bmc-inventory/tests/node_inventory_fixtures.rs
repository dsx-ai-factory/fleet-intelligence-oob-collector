// Copyright (c) 2025-2026, NVIDIA CORPORATION & AFFILIATES. All rights reserved.
//
// Licensed under the Apache License, Version 2.0 (the "License");
// you may not use this file except in compliance with the License.
// You may obtain a copy of the License at
//
//     http://www.apache.org/licenses/LICENSE-2.0
//
// Unless required by applicable law or agreed to in writing, software
// distributed under the License is distributed on an "AS IS" BASIS,
// WITHOUT WARRANTIES OR CONDITIONS OF ANY KIND, either express or implied.
// See the License for the specific language governing permissions and
// limitations under the License.
//
// SPDX-License-Identifier: Apache-2.0

use bmc_inventory::models::*;

fn ok_status(health_rollup: Option<&str>) -> InventoryStatus {
    InventoryStatus {
        conditions: None,
        health: Some("OK".to_string()),
        health_rollup: health_rollup.map(str::to_string),
        state: Some("Enabled".to_string()),
    }
}

/// OpenBMC-based NVIDIA BMCs report `"Conditions": []` rather than omitting it.
fn ok_status_with_conditions(health_rollup: Option<&str>) -> InventoryStatus {
    InventoryStatus {
        conditions: Some(Vec::new()),
        ..ok_status(health_rollup)
    }
}

fn links<const N: usize>(
    relations: [(&str, &[&str]); N],
) -> std::collections::BTreeMap<String, InventoryLink> {
    relations
        .into_iter()
        .map(|(relation, targets)| {
            let targets = targets
                .iter()
                .map(|target| InventoryLinkReference {
                    odata_id: (*target).into(),
                })
                .collect();
            (relation.to_string(), targets)
        })
        .collect()
}

#[test]
fn compute_node_inventory_serializes_expected_json() {
    // Representative compute BMC shape with synthetic identifiers.
    let mut inventory = NodeInventory {
        schema_version: INVENTORY_SCHEMA_VERSION.to_string(),
        collected_at: "2026-01-01T00:00:00Z".to_string(),
        enrolled_at: Some("2026-01-02T00:00:00Z".to_string()),
        kind: TargetKind::Compute,
        name: Some("node-01".to_string()),
        compute_zone: Some("us-west-dc1".to_string()),
        node_group: Some("gb200-training".to_string()),
        uuid: InventoryUuid::parse("9AEE71E4-6E80-11EE-A8AC-74D4DD550187"),
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
        managers: Vec::new(),
        systems: Vec::new(),
        chassis: Vec::new(),
        firmware: Vec::new(),
        fabrics: Vec::new(),
        domain_errors: Vec::new(),
        target_error: None,
        latency: Vec::new(),
    };
    inventory.managers.push(ManagerInventory {
        name: "Manager".to_string(),
        description: None,
        id: "BMC_0".to_string(),
        odata_id: Some("/redfish/v1/Managers/BMC_0".into()),
        uuid: Some("11111111-2222-3333-4444-555555555555".to_string()),
        manager_type: Some("BMC".to_string()),
        model: Some("OpenBMC".to_string()),
        firmware_version: Some("3.19.00".to_string()),
        status: Some(ok_status(Some("OK"))),
        links: std::collections::BTreeMap::from([(
            "ManagerForServers".to_string(),
            vec![InventoryLinkReference {
                odata_id: "/redfish/v1/Systems/System".into(),
            }],
        )]),
    });
    inventory.systems.push(SystemInventory {
        name: "System".to_string(),
        description: None,
        id: "System".to_string(),
        odata_id: Some("/redfish/v1/Systems/System".into()),
        uuid: InventoryUuid::parse("9AEE71E4-6E80-11EE-A8AC-74D4DD550187"),
        manufacturer: Some("NVIDIA".to_string()),
        model: Some("GB200".to_string()),
        sku: None,
        serial_number: Some("SN123".to_string()),
        bios_version: Some("3A21".to_string()),
        host_name: Some("node-a.example.com".to_string()),
        asset_tag: None,
        power_state: Some("On".to_string()),
        status: Some(ok_status(None)),
        processor_summary: Some(ProcessorSummary {
            count: Some(2),
            core_count: Some(72),
            model: Some("Grace CPU".to_string()),
            ..Default::default()
        }),
        memory_summary: Some(MemorySummary {
            total_system_memory_gib: Some(480.0),
            ..Default::default()
        }),
        secure_boot_enabled: Some(true),
        processors: vec![
            ProcessorInventory {
                name: "Processor".to_string(),
                description: None,
                id: "CPU1".to_string(),
                odata_id: Some("/redfish/v1/Systems/System/Processors/CPU1".into()),
                uuid: Some("22222222-3333-4444-5555-666666666666".to_string()),
                socket: Some("CPU Socket 1".to_string()),
                processor_type: Some("CPU".to_string()),
                processor_architecture: None,
                instruction_set: None,
                manufacturer: Some("NVIDIA".to_string()),
                model: Some("Grace CPU".to_string()),
                serial_number: None,
                part_number: None,
                location: Some(InventoryLocation {
                    part_location: Some(PartLocation {
                        service_label: Some("CPU Socket 1".to_string()),
                        location_type: Some("Slot".to_string()),
                        ..Default::default()
                    }),
                    placement: None,
                    part_location_context: Some("HGX_Chassis_0/ProcessorModule_0".to_string()),
                }),
                nvidia_gpu: None,
                max_speed_mhz: Some(3600),
                total_cores: Some(72),
                total_threads: Some(72),
                status: Some(ok_status(None)),
                links: Default::default(),
            },
            ProcessorInventory {
                name: "Processor".to_string(),
                description: None,
                id: "GPU1".to_string(),
                odata_id: Some("/redfish/v1/Systems/System/Processors/GPU1".into()),
                uuid: Some("33333333-4444-5555-6666-777777777777".to_string()),
                socket: None,
                processor_type: Some("GPU".to_string()),
                processor_architecture: None,
                instruction_set: None,
                manufacturer: Some("NVIDIA".to_string()),
                model: Some("B200".to_string()),
                serial_number: None,
                part_number: None,
                location: Some(InventoryLocation {
                    part_location: Some(PartLocation {
                        service_label: Some("GPU 1".to_string()),
                        ..Default::default()
                    }),
                    placement: None,
                    part_location_context: Some("HGX_Chassis_0/GPU_1".to_string()),
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
                max_speed_mhz: None,
                total_cores: None,
                total_threads: None,
                status: Some(ok_status(Some("OK"))),
                links: Default::default(),
            },
        ],
        links: Default::default(),
    });
    inventory.chassis.push(ChassisInventory {
        name: "Chassis".to_string(),
        description: None,
        id: "HGX_CPU_1".to_string(),
        odata_id: Some("/redfish/v1/Chassis/HGX_CPU_1".into()),
        chassis_type: Some("Module".to_string()),
        manufacturer: None,
        model: None,
        sku: None,
        serial_number: None,
        part_number: None,
        asset_tag: None,
        power_state: None,
        status: Some(ok_status(Some("OK"))),
        location: Some(InventoryLocation {
            part_location: Some(PartLocation {
                service_label: Some("Bay 1".to_string()),
                ..Default::default()
            }),
            placement: Some(Placement {
                rack: Some("R12".to_string()),
                row: Some("A".to_string()),
                rack_offset: Some(7),
                ..Default::default()
            }),
            part_location_context: None,
        }),
        pcie_devices: vec![PcieDeviceInventory {
            name: "PcieDevice".to_string(),
            description: None,
            id: "GPU1".to_string(),
            odata_id: Some("/redfish/v1/Chassis/HGX_CPU_1/PCIeDevices/GPU1".into()),
            uuid: None,
            device_type: Some("SingleFunction".to_string()),
            manufacturer: Some("NVIDIA".to_string()),
            model: Some("B200".to_string()),
            sku: None,
            serial_number: Some("GPU-SN456".to_string()),
            part_number: None,
            firmware_version: None,
            status: Some(ok_status(Some("OK"))),
            links: Default::default(),
        }],
        links: Default::default(),
    });
    inventory.firmware.push(FirmwareInventory {
        description: None,
        related_item: Some(vec![InventoryLinkReference {
            odata_id: "/redfish/v1/Managers/BMC_0".into(),
        }]),
        service_id: "FirmwareInventory".to_string(),
        id: "BMC".to_string(),
        odata_id: Some("/redfish/v1/UpdateService/FirmwareInventory/BMC".into()),
        name: "BMC".to_string(),
        version: Some("3.19.00".to_string()),
        release_date: None,
        status: Some(ok_status(Some("OK"))),
        links: Default::default(),
    });

    let actual = serde_json::to_value(inventory).expect("serialize inventory");
    let expected: serde_json::Value =
        serde_json::from_str(include_str!("fixtures/compute_node_inventory.json"))
            .expect("inventory contract example");
    assert_eq!(actual, expected);
}

#[test]
fn nvswitch_node_inventory_serializes_expected_json() {
    // Shape and non-identifying values from a captured NVIDIA NVSwitch tray BMC;
    // serial numbers, SKUs, UUIDs, and addresses are synthetic.
    let node_uuid = "5B1E2C3D-4E5F-4A6B-8C7D-9E0F1A2B3C4D";
    let switch = |index: u8, uuid: &str| SwitchInventory {
        name: format!("NVSwitch_{index} Resource"),
        description: None,
        id: format!("NVSwitch_{index}"),
        odata_id: Some(
            format!("/redfish/v1/Fabrics/MGX_NVLinkFabric_0/Switches/NVSwitch_{index}").into(),
        ),
        uuid: Some(uuid.to_string()),
        switch_type: Some("NVLink".to_string()),
        manufacturer: Some("NVIDIA".to_string()),
        // The captured BMC reports "NA" placeholders for these fields.
        model: Some("NA".to_string()),
        sku: None,
        serial_number: Some("NA".to_string()),
        part_number: Some("NA".to_string()),
        firmware_version: Some("35_2014_2184".to_string()),
        location: None,
        nvidia_switch: Some(NvidiaSwitchInventory {
            device_id: None,
            vendor_id: None,
            pcie_reference_clock_enabled: None,
            protected_pcie_mode_enabled: None,
            switch_isolation_mode: Some("SwitchCommunicationEnabled".to_string()),
        }),
        status: Some(ok_status_with_conditions(Some("OK"))),
        links: links([(
            "Chassis",
            &[&format!("/redfish/v1/Chassis/MGX_NVSwitch_{index}")],
        )]),
    };

    let inventory = NodeInventory {
        schema_version: INVENTORY_SCHEMA_VERSION.to_string(),
        collected_at: "2026-01-01T00:00:00Z".to_string(),
        enrolled_at: Some("2026-01-02T00:00:00Z".to_string()),
        kind: TargetKind::NvSwitch,
        name: Some("nvswitch-01".to_string()),
        compute_zone: Some("us-west-dc1".to_string()),
        node_group: Some("gb200-training".to_string()),
        uuid: InventoryUuid::parse(node_uuid),
        source: InventorySourceInfo {
            source_type: "redfish".to_string(),
            mac: "00:11:22:33:44:66".to_string(),
            address: "10.0.0.2".to_string(),
            host_name: Some("bmc-nvswitch-a.example.com".to_string()),
            redfish_version: Some("1.17.0".to_string()),
            service_uuid: Some("6c2f3d4e-5f60-4b7c-9d8e-0f1a2b3c4d5e".to_string()),
            vendor: Some("NVIDIA".to_string()),
        },
        primary_system_id: Some("System_0".to_string()),
        managers: vec![ManagerInventory {
            name: "OpenBmc Manager".to_string(),
            description: Some("Baseboard Management Controller".to_string()),
            id: "BMC_0".to_string(),
            odata_id: Some("/redfish/v1/Managers/BMC_0".into()),
            uuid: Some("7d304e5f-6071-4c8d-ae9f-1a2b3c4d5e6f".to_string()),
            manager_type: Some("BMC".to_string()),
            model: Some("OpenBmc".to_string()),
            firmware_version: Some("88.0002.1336".to_string()),
            status: Some(ok_status_with_conditions(Some("OK"))),
            links: links([
                (
                    "ActiveSoftwareImage",
                    &["/redfish/v1/UpdateService/FirmwareInventory/MGX_FW_BMC_0"],
                ),
                ("ManagerForChassis", &["/redfish/v1/Chassis/BMC_eeprom"]),
                ("ManagerForServers", &["/redfish/v1/Systems/System_0"]),
                ("ManagerInChassis", &["/redfish/v1/Chassis/BMC_eeprom"]),
                (
                    "SoftwareImages",
                    &["/redfish/v1/UpdateService/FirmwareInventory/MGX_FW_BMC_0"],
                ),
            ]),
        }],
        systems: vec![SystemInventory {
            name: "System_0".to_string(),
            description: Some("Computer System".to_string()),
            id: "System_0".to_string(),
            odata_id: Some("/redfish/v1/Systems/System_0".into()),
            // The tray's system reports no UUID, so it carries the node UUID.
            uuid: InventoryUuid::parse(node_uuid),
            manufacturer: None,
            model: None,
            sku: None,
            serial_number: None,
            bios_version: None,
            host_name: None,
            asset_tag: None,
            power_state: Some("On".to_string()),
            status: Some(ok_status_with_conditions(None)),
            processor_summary: Some(ProcessorSummary {
                count: Some(0),
                ..Default::default()
            }),
            memory_summary: Some(MemorySummary {
                total_system_memory_gib: Some(0.0),
                ..Default::default()
            }),
            secure_boot_enabled: None,
            processors: Vec::new(),
            links: links([
                ("Chassis", &["/redfish/v1/Chassis/BMC_eeprom"]),
                ("ManagedBy", &["/redfish/v1/Managers/BMC_0"]),
            ]),
        }],
        chassis: vec![ChassisInventory {
            name: "MGX_NVSwitch_0".to_string(),
            description: None,
            id: "MGX_NVSwitch_0".to_string(),
            odata_id: Some("/redfish/v1/Chassis/MGX_NVSwitch_0".into()),
            chassis_type: Some("Component".to_string()),
            manufacturer: Some("NVIDIA".to_string()),
            model: Some("N5400_LD".to_string()),
            sku: Some("SKU-NVS-01".to_string()),
            serial_number: Some("NVS-SN-0001".to_string()),
            part_number: Some("NVS-PN-0001".to_string()),
            asset_tag: None,
            power_state: Some("On".to_string()),
            status: Some(ok_status_with_conditions(Some("OK"))),
            location: Some(InventoryLocation {
                part_location: Some(PartLocation {
                    location_type: Some("Embedded".to_string()),
                    ..Default::default()
                }),
                placement: None,
                part_location_context: None,
            }),
            pcie_devices: Vec::new(),
            links: links([
                ("ComputerSystems", &["/redfish/v1/Systems/System_0"]),
                ("ManagedBy", &["/redfish/v1/Managers/BMC_0"]),
                (
                    "Switches",
                    &["/redfish/v1/Fabrics/MGX_NVLinkFabric_0/Switches/NVSwitch_0"],
                ),
            ]),
        }],
        firmware: vec![FirmwareInventory {
            description: Some("Other image".to_string()),
            related_item: Some(vec![InventoryLinkReference {
                odata_id: "/redfish/v1/Chassis/MGX_NVSwitch_0".into(),
            }]),
            service_id: "FirmwareInventory".to_string(),
            id: "MGX_FW_NVSwitch_0".to_string(),
            odata_id: Some("/redfish/v1/UpdateService/FirmwareInventory/MGX_FW_NVSwitch_0".into()),
            name: "Software Inventory".to_string(),
            version: Some("35_2014_2184".to_string()),
            release_date: None,
            status: Some(ok_status_with_conditions(Some("OK"))),
            links: Default::default(),
        }],
        fabrics: vec![FabricInventory {
            name: "MGX_NVLinkFabric_0 Resource".to_string(),
            description: None,
            id: "MGX_NVLinkFabric_0".to_string(),
            odata_id: Some("/redfish/v1/Fabrics/MGX_NVLinkFabric_0".into()),
            // The captured fabric reports no UUID, status, or links.
            uuid: None,
            fabric_type: Some("NVLink".to_string()),
            status: None,
            switches: vec![
                switch(0, "8e415f60-7182-4d9e-bf0a-2b3c4d5e6f70"),
                switch(1, "9f526071-8293-4eaf-8a1b-3c4d5e6f7081"),
            ],
            links: Default::default(),
        }],
        domain_errors: Vec::new(),
        target_error: None,
        latency: Vec::new(),
    };

    let actual = serde_json::to_value(inventory).expect("serialize inventory");
    let expected: serde_json::Value =
        serde_json::from_str(include_str!("fixtures/nvswitch_node_inventory.json"))
            .expect("inventory contract example");
    assert_eq!(actual, expected);
}
