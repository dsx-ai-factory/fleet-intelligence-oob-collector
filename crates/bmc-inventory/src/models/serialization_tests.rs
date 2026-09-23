use serde_json::json;

use super::*;

#[test]
fn inventory_condition_serializes_diagnostic_fields() {
    let condition = InventoryCondition {
        timestamp: Some("2026-08-25T20:51:47Z".to_string()),
        message_id: "ResourceEvent.1.0.ResourceErrorsDetected".to_string(),
        message_args: Some(vec!["CPU_1 Firmware".into(), "EC_STRAP_MISMATCH".into()]),
        message: Some("The resource property CPU_1 Firmware has detected errors.".into()),
        severity: Some("Critical".to_string()),
        resolution: Some("Reset the component and collect logs.".into()),
        condition_type: Some("Alert".to_string()),
        origin_of_condition: Some(InventoryReference {
            odata_id: "/redfish/v1/Chassis/HGX_CPU_1".to_string().into(),
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
                "odataId": "/redfish/v1/Chassis/HGX_CPU_1"
            }
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
        target_error: Some("malformed system UUID".into()),
    };

    let value = serde_json::to_value(inventory).expect("json");

    assert!(value["uuid"].is_null());
    assert!(value.get("name").is_none());
    assert!(value.get("computeZone").is_none());
    assert!(value.get("nodeGroup").is_none());
    assert!(value["enrolledAt"].is_null());
    assert_eq!(value["targetError"], "malformed system UUID");
}
