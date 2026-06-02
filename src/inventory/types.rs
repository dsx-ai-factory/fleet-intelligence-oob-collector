use std::fmt;

use serde::{Serialize, Serializer};
use uuid::Uuid;

pub const INVENTORY_SCHEMA_VERSION: &str = "inventory.v1alpha1";

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SystemUuid(Uuid);

impl SystemUuid {
    pub fn parse(value: &str) -> Option<Self> {
        Uuid::parse_str(value.trim()).ok().map(Self)
    }
}

impl fmt::Display for SystemUuid {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.0.fmt(f)
    }
}

impl Serialize for SystemUuid {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        serializer.serialize_str(&self.to_string())
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ChassisSerial(String);

impl ChassisSerial {
    pub fn parse(value: &str) -> Option<Self> {
        let value = value.trim();
        if value.is_empty() {
            None
        } else {
            Some(Self(value.to_string()))
        }
    }
}

impl fmt::Display for ChassisSerial {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

impl Serialize for ChassisSerial {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        serializer.serialize_str(&self.0)
    }
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct NodeInventory {
    pub schema_version: String,
    pub collected_at: String,
    pub node: Option<NodeInfo>,
    pub source: InventorySourceInfo,
    pub manager: Option<ManagerInventory>,
    pub chassis: Vec<ChassisInventory>,
    pub devices: Vec<DeviceInventory>,
    pub firmware: Vec<FirmwareInventory>,
    pub errors: Vec<InventoryError>,
    pub collection_error: Option<String>,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct NodeInfo {
    pub system_id: String,
    pub odata_id: Option<String>,
    pub system_uuid: Option<SystemUuid>,
    pub manufacturer: Option<String>,
    pub model: Option<String>,
    pub sku: Option<String>,
    pub serial_number: Option<String>,
    pub bios_version: Option<String>,
    pub host_name: Option<String>,
    pub asset_tag: Option<String>,
    pub power_state: String,
    pub status_state: Option<String>,
    pub health: Option<String>,
    pub cpu_count: Option<i64>,
    pub cpu_core_count: Option<i64>,
    pub cpu_model: Option<String>,
    pub memory_gib: Option<f64>,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct InventorySourceInfo {
    pub source_type: String,
    pub target: String,
    pub address: String,
    pub redfish_version: Option<String>,
    pub service_uuid: Option<String>,
    pub vendor: Option<String>,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ManagerInventory {
    pub id: String,
    pub odata_id: Option<String>,
    pub uuid: Option<String>,
    pub manager_type: Option<String>,
    pub model: Option<String>,
    pub firmware_version: Option<String>,
    pub status_state: Option<String>,
    pub health: Option<String>,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ChassisInventory {
    pub id: String,
    pub odata_id: Option<String>,
    pub chassis_type: Option<String>,
    pub manufacturer: Option<String>,
    pub model: Option<String>,
    pub sku: Option<String>,
    pub serial_number: Option<ChassisSerial>,
    pub part_number: Option<String>,
    pub asset_tag: Option<String>,
    pub power_state: Option<String>,
    pub status_state: Option<String>,
    pub health: Option<String>,
    pub location: Option<ChassisLocation>,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ChassisLocation {
    pub service_label: Option<String>,
    pub rack: Option<String>,
    pub row: Option<String>,
    pub room: Option<String>,
    pub rack_offset: Option<i64>,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DeviceInventory {
    pub id: String,
    pub odata_id: Option<String>,
    pub uuid: Option<String>,
    pub source: String,
    pub parent_type: Option<String>,
    pub parent_id: Option<String>,
    pub device_type: Option<String>,
    pub manufacturer: Option<String>,
    pub model: Option<String>,
    pub sku: Option<String>,
    pub serial_number: Option<String>,
    pub part_number: Option<String>,
    pub firmware_version: Option<String>,
    pub status_state: Option<String>,
    pub health: Option<String>,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct FirmwareInventory {
    pub service_id: String,
    pub id: String,
    pub odata_id: Option<String>,
    pub name: String,
    pub version: Option<String>,
    pub release_date: Option<String>,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct InventoryError {
    pub domain: String,
    pub resource: Option<String>,
    pub message: String,
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;

    #[test]
    fn node_inventory_serializes_as_backend_payload() {
        let inventory = NodeInventory {
            schema_version: INVENTORY_SCHEMA_VERSION.to_string(),
            collected_at: "2026-01-01T00:00:00Z".to_string(),
            node: Some(NodeInfo {
                system_id: "System".to_string(),
                odata_id: Some("/redfish/v1/Systems/System".to_string()),
                system_uuid: SystemUuid::parse("9AEE71E4-6E80-11EE-A8AC-74D4DD550187"),
                manufacturer: Some("NVIDIA".to_string()),
                model: Some("GB200".to_string()),
                sku: Some("SKU-GB200".to_string()),
                serial_number: Some("SN123".to_string()),
                bios_version: Some("3A21".to_string()),
                host_name: Some("node-a.example.com".to_string()),
                asset_tag: Some("rack-12-tray-3".to_string()),
                power_state: "on".to_string(),
                status_state: Some("enabled".to_string()),
                health: Some("ok".to_string()),
                cpu_count: Some(1),
                cpu_core_count: Some(72),
                cpu_model: Some("Grace".to_string()),
                memory_gib: Some(480.0),
            }),
            source: InventorySourceInfo {
                source_type: "redfish".to_string(),
                target: "node-a".to_string(),
                address: "10.0.0.1".to_string(),
                redfish_version: Some("1.15.1".to_string()),
                service_uuid: Some("870155dd-d474-aca8-11ee-6e809af87c0c".to_string()),
                vendor: Some("NVIDIA".to_string()),
            },
            manager: Some(ManagerInventory {
                id: "BMC_0".to_string(),
                odata_id: Some("/redfish/v1/Managers/BMC_0".to_string()),
                uuid: Some("11111111-2222-3333-4444-555555555555".to_string()),
                manager_type: Some("bmc".to_string()),
                model: Some("OpenBMC".to_string()),
                firmware_version: Some("3.19.00".to_string()),
                status_state: Some("enabled".to_string()),
                health: Some("ok".to_string()),
            }),
            chassis: vec![ChassisInventory {
                id: "BMC_0".to_string(),
                odata_id: Some("/redfish/v1/Chassis/BMC_0".to_string()),
                chassis_type: Some("module".to_string()),
                manufacturer: Some("NVIDIA".to_string()),
                model: Some("BMC".to_string()),
                sku: Some("CH-SKU".to_string()),
                serial_number: ChassisSerial::parse("CH123"),
                part_number: Some("PN123".to_string()),
                asset_tag: Some("rack-12".to_string()),
                power_state: Some("on".to_string()),
                status_state: Some("enabled".to_string()),
                health: Some("ok".to_string()),
                location: Some(crate::inventory::types::ChassisLocation {
                    service_label: Some("Bay 1".to_string()),
                    rack: Some("R12".to_string()),
                    row: Some("A".to_string()),
                    room: None,
                    rack_offset: Some(7),
                }),
            }],
            devices: vec![DeviceInventory {
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
            firmware: vec![FirmwareInventory {
                service_id: "FirmwareInventory".to_string(),
                id: "BMC".to_string(),
                odata_id: Some("/redfish/v1/UpdateService/FirmwareInventory/BMC".to_string()),
                name: "BMC".to_string(),
                version: Some("3.19.00".to_string()),
                release_date: Some("2026-01-01T00:00:00Z".to_string()),
            }],
            errors: vec![InventoryError {
                domain: "devices.processors".to_string(),
                resource: Some("System".to_string()),
                message: "timeout".to_string(),
            }],
            collection_error: None,
        };

        let value = serde_json::to_value(inventory).expect("json");

        assert_eq!(
            value,
            json!({
                "schemaVersion": "inventory.v1alpha1",
                "collectedAt": "2026-01-01T00:00:00Z",
                "node": {
                    "systemId": "System",
                    "odataId": "/redfish/v1/Systems/System",
                    "systemUuid": "9aee71e4-6e80-11ee-a8ac-74d4dd550187",
                    "manufacturer": "NVIDIA",
                    "model": "GB200",
                    "sku": "SKU-GB200",
                    "serialNumber": "SN123",
                    "biosVersion": "3A21",
                    "hostName": "node-a.example.com",
                    "assetTag": "rack-12-tray-3",
                    "powerState": "on",
                    "statusState": "enabled",
                    "health": "ok",
                    "cpuCount": 1,
                    "cpuCoreCount": 72,
                    "cpuModel": "Grace",
                    "memoryGib": 480.0
                },
                "source": {
                    "sourceType": "redfish",
                    "target": "node-a",
                    "address": "10.0.0.1",
                    "redfishVersion": "1.15.1",
                    "serviceUuid": "870155dd-d474-aca8-11ee-6e809af87c0c",
                    "vendor": "NVIDIA"
                },
                "manager": {
                    "id": "BMC_0",
                    "odataId": "/redfish/v1/Managers/BMC_0",
                    "uuid": "11111111-2222-3333-4444-555555555555",
                    "managerType": "bmc",
                    "model": "OpenBMC",
                    "firmwareVersion": "3.19.00",
                    "statusState": "enabled",
                    "health": "ok"
                },
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
                    "statusState": "enabled",
                    "health": "ok",
                    "location": {
                        "serviceLabel": "Bay 1",
                        "rack": "R12",
                        "row": "A",
                        "room": null,
                        "rackOffset": 7
                    }
                }],
                "devices": [{
                    "id": "GPU0",
                    "odataId": "/redfish/v1/Systems/System/Processors/GPU0",
                    "uuid": "22222222-3333-4444-5555-666666666666",
                    "source": "processor",
                    "parentType": "system",
                    "parentId": "System",
                    "deviceType": "gpu",
                    "manufacturer": "NVIDIA",
                    "model": "B200",
                    "sku": null,
                    "serialNumber": "GPU-SN123",
                    "partNumber": "GPU-PN123",
                    "firmwareVersion": "96.00.5E.00.01",
                    "statusState": "enabled",
                    "health": "ok"
                }],
                "firmware": [{
                    "serviceId": "FirmwareInventory",
                    "id": "BMC",
                    "odataId": "/redfish/v1/UpdateService/FirmwareInventory/BMC",
                    "name": "BMC",
                    "version": "3.19.00",
                    "releaseDate": "2026-01-01T00:00:00Z"
                }],
                "errors": [{
                    "domain": "devices.processors",
                    "resource": "System",
                    "message": "timeout"
                }],
                "collectionError": null
            })
        );
    }
}
