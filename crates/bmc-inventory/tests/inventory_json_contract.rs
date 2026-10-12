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

use bmc_inventory::models::{
    FabricInventory, INVENTORY_SCHEMA_VERSION, InventoryCondition, InventoryReference,
    InventorySourceInfo, InventoryStatus, NodeInventory, NvidiaSwitchInventory, SwitchInventory,
    TargetKind,
};
use serde_json::json;

fn failed_inventory() -> NodeInventory {
    NodeInventory {
        schema_version: INVENTORY_SCHEMA_VERSION.to_string(),
        collected_at: "2026-01-01T00:00:00Z".to_string(),
        enrolled_at: None,
        kind: TargetKind::Unknown,
        name: None,
        compute_zone: None,
        node_group: None,
        source: InventorySourceInfo {
            source_type: "redfish".to_string(),
            mac: "00:11:22:33:44:55".to_string(),
            address: "10.0.0.1".to_string(),
            host_name: None,
            redfish_version: None,
            service_uuid: None,
            vendor: None,
        },
        managers: Vec::new(),
        systems: Vec::new(),
        chassis: Vec::new(),
        firmware: Vec::new(),
        fabrics: Vec::new(),
        domain_errors: Vec::new(),
        target_error: Some("connection refused".into()),
        latency: Vec::new(),
    }
}

#[test]
fn condition_serializes_diagnostic_fields() {
    // The saved compute responses have no Conditions; cover their wire names separately.
    let condition = InventoryCondition {
        timestamp: Some("2026-08-25T20:51:47Z".to_string()),
        message_id: "ResourceEvent.1.0.ResourceErrorsDetected".to_string(),
        message_args: Some(vec!["CPU_1 Firmware".into(), "EC_STRAP_MISMATCH".into()]),
        message: Some("CPU_1 Firmware has detected errors".into()),
        severity: Some("Critical".to_string()),
        resolution: Some("Reset the component".into()),
        condition_type: Some("Alert".to_string()),
        origin_of_condition: Some(InventoryReference {
            odata_id: "/redfish/v1/Chassis/HGX_CPU_1".into(),
        }),
    };

    assert_eq!(
        serde_json::to_value(condition).expect("serialize condition"),
        json!({
            "timestamp": "2026-08-25T20:51:47Z",
            "messageId": "ResourceEvent.1.0.ResourceErrorsDetected",
            "messageArgs": ["CPU_1 Firmware", "EC_STRAP_MISMATCH"],
            "message": "CPU_1 Firmware has detected errors",
            "severity": "Critical",
            "resolution": "Reset the component",
            "conditionType": "Alert",
            "originOfCondition": {
                "odataId": "/redfish/v1/Chassis/HGX_CPU_1"
            }
        })
    );
}

#[test]
fn target_error_inventory_uses_mac_without_node_identity() {
    let actual =
        serde_json::to_value(failed_inventory()).expect("serialize failed target inventory");
    assert_eq!(
        actual,
        json!({
            "schemaVersion": "inventory.v1alpha1",
            "collectedAt": "2026-01-01T00:00:00Z",
            "enrolledAt": null,
            "kind": "unknown",
            "source": {
                "sourceType": "redfish",
                "mac": "00:11:22:33:44:55",
                "address": "10.0.0.1",
                "hostName": null,
                "redfishVersion": null,
                "serviceUuid": null,
                "vendor": null
            },
            "managers": [],
            "systems": [],
            "chassis": [],
            "firmware": [],
            "fabrics": [],
            "domainErrors": [],
            "targetError": "connection refused",
            "latency": []
        })
    );
}

#[test]
fn fabric_serializes_backend_switch_contract() {
    let status = || {
        Some(InventoryStatus {
            conditions: None,
            health: Some("OK".to_string()),
            health_rollup: Some("OK".to_string()),
            state: Some("Enabled".to_string()),
        })
    };
    let fabric = FabricInventory {
        name: "NVLink Fabric".to_string(),
        description: None,
        id: "NVLink".to_string(),
        odata_id: Some("/redfish/v1/Fabrics/NVLink".into()),
        uuid: None,
        fabric_type: Some("NVLink".to_string()),
        status: status(),
        switches: vec![SwitchInventory {
            name: "NVSwitch 0".to_string(),
            description: None,
            id: "NVSwitch_0".to_string(),
            odata_id: Some("/redfish/v1/Fabrics/NVLink/Switches/NVSwitch_0".into()),
            uuid: None,
            switch_type: Some("NVLink".to_string()),
            manufacturer: Some("NVIDIA".to_string()),
            model: Some("NVSwitch".to_string()),
            sku: None,
            serial_number: Some("SW-SERIAL".to_string()),
            part_number: None,
            firmware_version: Some("1.2.3".to_string()),
            location: None,
            nvidia_switch: Some(NvidiaSwitchInventory {
                device_id: Some("0x22A3".to_string()),
                vendor_id: Some("0x10DE".to_string()),
                pcie_reference_clock_enabled: Some(true),
                protected_pcie_mode_enabled: Some(false),
                switch_isolation_mode: Some("SwitchCommunicationEnabled".to_string()),
            }),
            status: status(),
            links: Default::default(),
        }],
        links: Default::default(),
    };

    let status_json = json!({
        "conditions": null,
        "health": "OK",
        "healthRollup": "OK",
        "state": "Enabled"
    });
    assert_eq!(
        serde_json::to_value(fabric).expect("serialize fabric"),
        json!({
            "name": "NVLink Fabric",
            "description": null,
            "id": "NVLink",
            "odataId": "/redfish/v1/Fabrics/NVLink",
            "uuid": null,
            "fabricType": "NVLink",
            "status": status_json,
            "switches": [{
                "name": "NVSwitch 0",
                "description": null,
                "id": "NVSwitch_0",
                "odataId": "/redfish/v1/Fabrics/NVLink/Switches/NVSwitch_0",
                "uuid": null,
                "switchType": "NVLink",
                "manufacturer": "NVIDIA",
                "model": "NVSwitch",
                "sku": null,
                "serialNumber": "SW-SERIAL",
                "partNumber": null,
                "firmwareVersion": "1.2.3",
                "location": null,
                "nvidiaSwitch": {
                    "deviceId": "0x22A3",
                    "vendorId": "0x10DE",
                    "pcieReferenceClockEnabled": true,
                    "protectedPcieModeEnabled": false,
                    "switchIsolationMode": "SwitchCommunicationEnabled"
                },
                "status": status_json,
                "links": {}
            }],
            "links": {}
        })
    );
}
