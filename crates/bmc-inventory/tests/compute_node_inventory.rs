use bmc_inventory::models::*;

fn ok_status(health_rollup: Option<&str>) -> InventoryStatus {
    InventoryStatus {
        conditions: None,
        health: Some("OK".to_string()),
        health_rollup: health_rollup.map(str::to_string),
        state: Some("Enabled".to_string()),
    }
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
