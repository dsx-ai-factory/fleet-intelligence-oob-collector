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

use bmc_error::{SanitizedErrorMessage, SanitizedUriReference};
use serde::Serialize;

use super::InventoryUuid;

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct InventorySourceInfo {
    pub source_type: String,
    pub mac: String,
    pub address: String,
    pub redfish_version: Option<String>,
    pub service_uuid: Option<String>,
    pub vendor: Option<String>,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ManagerInventory {
    pub name: String,
    pub description: Option<String>,
    pub id: String,
    pub odata_id: Option<SanitizedUriReference>,
    pub uuid: Option<String>,
    pub manager_type: Option<String>,
    pub model: Option<String>,
    pub firmware_version: Option<String>,
    pub status: Option<InventoryStatus>,
    pub links: std::collections::BTreeMap<String, InventoryLink>,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SystemInventory {
    pub name: String,
    pub description: Option<String>,
    pub id: String,
    pub odata_id: Option<SanitizedUriReference>,
    pub uuid: Option<InventoryUuid>,
    pub manufacturer: Option<String>,
    pub model: Option<String>,
    pub sku: Option<String>,
    pub serial_number: Option<String>,
    pub bios_version: Option<String>,
    pub host_name: Option<String>,
    pub asset_tag: Option<String>,
    pub power_state: Option<String>,
    pub status: Option<InventoryStatus>,
    pub processor_summary: Option<ProcessorSummary>,
    pub memory_summary: Option<MemorySummary>,
    pub secure_boot_enabled: Option<bool>,
    pub processors: Vec<ProcessorInventory>,
    pub memory: Vec<MemoryInventory>,
    pub storage: Vec<StorageInventory>,
    pub links: std::collections::BTreeMap<String, InventoryLink>,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ProcessorInventory {
    pub name: String,
    pub description: Option<String>,
    pub id: String,
    pub odata_id: Option<SanitizedUriReference>,
    pub uuid: Option<String>,
    pub socket: Option<String>,
    pub processor_type: Option<String>,
    pub processor_architecture: Option<String>,
    pub instruction_set: Option<String>,
    pub manufacturer: Option<String>,
    pub model: Option<String>,
    pub serial_number: Option<String>,
    pub part_number: Option<String>,
    pub location: Option<InventoryLocation>,
    pub nvidia_gpu: Option<NvidiaGpuInventory>,
    pub max_speed_mhz: Option<i64>,
    pub total_cores: Option<i64>,
    pub total_threads: Option<i64>,
    pub status: Option<InventoryStatus>,
    pub links: std::collections::BTreeMap<String, InventoryLink>,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct NvidiaGpuInventory {
    pub mig_mode_enabled: Option<bool>,
    pub default_boost_clock_speed_mhz: Option<i64>,
    pub max_boost_clock_speed_mhz: Option<i64>,
    pub cc_mode_enabled: Option<bool>,
    pub cc_dev_mode_enabled: Option<bool>,
    pub protected_pcie_mode_enabled: Option<bool>,
    pub egm_mode_enabled: Option<bool>,
    pub total_nv_links: Option<i64>,
    pub nvlink_topology: Option<NvlinkTopologyInventory>,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct NvlinkTopologyInventory {
    pub tray_serial_number: Option<String>,
    pub tray_slot_number: Option<i64>,
    pub tray_slot_index: Option<i64>,
    pub chassis_serial_number: Option<String>,
    pub host_id: Option<i64>,
    pub module_id: Option<i64>,
    pub system_guid: Option<String>,
    pub ib_guid: Option<String>,
    pub lsb_ib_guid: Option<String>,
    pub peer_type: Option<String>,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ChassisInventory {
    pub name: String,
    pub description: Option<String>,
    pub id: String,
    pub odata_id: Option<SanitizedUriReference>,
    pub chassis_type: Option<String>,
    pub manufacturer: Option<String>,
    pub model: Option<String>,
    pub sku: Option<String>,
    pub serial_number: Option<String>,
    pub part_number: Option<String>,
    pub asset_tag: Option<String>,
    pub power_state: Option<String>,
    pub status: Option<InventoryStatus>,
    pub location: Option<InventoryLocation>,
    pub pcie_devices: Vec<PcieDeviceInventory>,
    pub power_supplies: Vec<PowerSupplyInventory>,
    pub links: std::collections::BTreeMap<String, InventoryLink>,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct InventoryLocation {
    pub part_location: Option<PartLocation>,
    pub placement: Option<Placement>,
    pub part_location_context: Option<String>,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PcieDeviceInventory {
    pub name: String,
    pub description: Option<String>,
    pub id: String,
    pub odata_id: Option<SanitizedUriReference>,
    pub uuid: Option<String>,
    pub device_type: Option<String>,
    pub manufacturer: Option<String>,
    pub model: Option<String>,
    pub sku: Option<String>,
    pub serial_number: Option<String>,
    pub part_number: Option<String>,
    pub firmware_version: Option<String>,
    pub status: Option<InventoryStatus>,
    pub links: std::collections::BTreeMap<String, InventoryLink>,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct FirmwareInventory {
    pub description: Option<String>,
    pub related_item: Option<Vec<InventoryLinkReference>>,
    pub service_id: String,
    pub id: String,
    pub odata_id: Option<SanitizedUriReference>,
    pub name: String,
    pub version: Option<String>,
    pub release_date: Option<String>,
    pub status: Option<InventoryStatus>,
    pub links: std::collections::BTreeMap<String, InventoryLink>,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct FabricInventory {
    pub name: String,
    pub description: Option<String>,
    pub id: String,
    pub odata_id: Option<SanitizedUriReference>,
    pub uuid: Option<String>,
    pub fabric_type: Option<String>,
    pub status: Option<InventoryStatus>,
    pub switches: Vec<SwitchInventory>,
    pub links: std::collections::BTreeMap<String, InventoryLink>,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SwitchInventory {
    pub name: String,
    pub description: Option<String>,
    pub id: String,
    pub odata_id: Option<SanitizedUriReference>,
    pub uuid: Option<String>,
    pub switch_type: Option<String>,
    pub manufacturer: Option<String>,
    pub model: Option<String>,
    pub sku: Option<String>,
    pub serial_number: Option<String>,
    pub part_number: Option<String>,
    pub firmware_version: Option<String>,
    pub location: Option<InventoryLocation>,
    pub nvidia_switch: Option<NvidiaSwitchInventory>,
    pub status: Option<InventoryStatus>,
    pub links: std::collections::BTreeMap<String, InventoryLink>,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct NvidiaSwitchInventory {
    pub device_id: Option<String>,
    pub vendor_id: Option<String>,
    pub pcie_reference_clock_enabled: Option<bool>,
    pub protected_pcie_mode_enabled: Option<bool>,
    pub switch_isolation_mode: Option<String>,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct InventoryStatus {
    pub conditions: Option<Vec<InventoryCondition>>,
    pub health: Option<String>,
    pub health_rollup: Option<String>,
    pub state: Option<String>,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct InventoryCondition {
    pub timestamp: Option<String>,
    pub message_id: String,
    pub message_args: Option<Vec<SanitizedErrorMessage>>,
    pub message: Option<SanitizedErrorMessage>,
    pub severity: Option<String>,
    pub resolution: Option<SanitizedErrorMessage>,
    pub condition_type: Option<String>,
    pub origin_of_condition: Option<InventoryReference>,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct InventoryReference {
    pub odata_id: SanitizedUriReference,
}

#[derive(Debug, Default, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ProcessorSummary {
    pub count: Option<i64>,
    pub core_count: Option<i64>,
    pub model: Option<String>,
    pub logical_processor_count: Option<i64>,
    pub threading_enabled: Option<bool>,
    pub status: Option<InventoryStatus>,
}

#[derive(Debug, Default, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct MemorySummary {
    #[serde(rename = "totalSystemMemoryGiB")]
    pub total_system_memory_gib: Option<f64>,
    #[serde(rename = "totalSystemPersistentMemoryGiB")]
    pub total_system_persistent_memory_gib: Option<f64>,
    pub memory_mirroring: Option<String>,
    pub status: Option<InventoryStatus>,
}

/// References for one relationship. Product inventory always emits an array,
/// regardless of whether Redfish represented the relationship as one or many.
pub type InventoryLink = Vec<InventoryLinkReference>;

#[derive(Debug, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct InventoryLinkReference {
    pub odata_id: SanitizedUriReference,
}

#[derive(Debug, Default, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PartLocation {
    pub service_label: Option<String>,
    pub location_type: Option<String>,
    pub location_ordinal_value: Option<i64>,
    pub reference: Option<String>,
    pub orientation: Option<String>,
}

#[derive(Debug, Default, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Placement {
    pub rack: Option<String>,
    pub row: Option<String>,
    pub room: Option<String>,
    pub rack_offset: Option<i64>,
    pub rack_offset_units: Option<String>,
    pub additional_info: Option<String>,
    pub facility_name: Option<String>,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct MemoryInventory {
    pub id: String,
    pub system_id: String,
    pub odata_id: Option<SanitizedUriReference>,
    pub memory_device_type: Option<String>,
    pub capacity_mib: Option<i64>,
    pub serial_number: Option<String>,
    pub part_number: Option<String>,
    pub manufacturer: Option<String>,
    pub device_locator: Option<String>,
    pub location: Option<InventoryLocation>,
    pub status: Option<InventoryStatus>,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct StorageInventory {
    pub id: String,
    pub system_id: String,
    pub odata_id: Option<SanitizedUriReference>,
    pub drives: Vec<DriveInventory>,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DriveInventory {
    pub id: String,
    pub storage_id: String,
    pub system_id: String,
    pub odata_id: Option<SanitizedUriReference>,
    pub model: Option<String>,
    pub serial_number: Option<String>,
    pub capacity_bytes: Option<i64>,
    pub protocol: Option<String>,
    pub media_type: Option<String>,
    pub revision: Option<String>,
    pub status: Option<InventoryStatus>,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PowerSupplyInventory {
    pub id: String,
    pub chassis_id: String,
    pub odata_id: Option<SanitizedUriReference>,
    pub model: Option<String>,
    pub serial_number: Option<String>,
    pub part_number: Option<String>,
    pub firmware_version: Option<String>,
    pub power_capacity_watts: Option<f64>,
    pub status: Option<InventoryStatus>,
}
