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
use nv_redfish::fabric::{Fabric, Switch};
use nv_redfish::oem::nvidia::NvidiaProcessor;
use nv_redfish::oem::nvidia::schema::nvidia_processor::NvidiaGpu;
use nv_redfish::oem::nvidia::schema::nvidia_switch::NvidiaSwitch as NvidiaSwitchSchema;
use nv_redfish::{Bmc, ServiceRoot};

use super::links::{
    chassis_links, fabric_links, firmware_links, link_targets, manager_links, pcie_device_links,
    processor_links, switch_links, system_links,
};
use super::location::location_data;
use super::status::{redfish_enum_string, status_data};
use crate::models::{
    ChassisInventory, DomainError, DriveInventory, FabricInventory, FirmwareInventory,
    InventoryUuid, ManagerInventory, MemoryInventory, MemorySummary, NvidiaGpuInventory,
    NvidiaSwitchInventory, NvlinkTopologyInventory, PcieDeviceInventory, PowerSupplyInventory,
    ProcessorInventory, ProcessorSummary, StorageInventory, SwitchInventory, SystemInventory,
};
use crate::source::{RedfishReport, ServiceRootData, inventory_domain};

pub(super) async fn collect_report_from_root<B: Bmc>(
    root: &ServiceRoot<B>,
    bmc: &B,
) -> Result<RedfishReport, String> {
    let raw_root = root.root.as_ref();
    let service_root = ServiceRootData {
        redfish_version: raw_root.redfish_version.clone(),
        uuid: raw_root.uuid.flatten().map(|uuid| uuid.to_string()),
        vendor: raw_root.vendor.clone().flatten(),
        product: raw_root.product.clone().flatten(),
    };

    let mut errors = Vec::new();
    let managers = collect_domain(
        &mut errors,
        inventory_domain::MANAGERS,
        collect_managers(root).await,
    );
    let systems_result = collect_systems(root, &mut errors).await;
    let systems = collect_domain(&mut errors, inventory_domain::SYSTEMS, systems_result);
    let chassis_result = collect_chassis(root, bmc, &mut errors).await;
    let chassis = collect_domain(&mut errors, inventory_domain::CHASSIS, chassis_result);
    let firmware = collect_domain(
        &mut errors,
        inventory_domain::FIRMWARE,
        collect_firmware(root).await,
    );
    let fabrics_result = collect_fabrics(root, &mut errors).await;
    let fabrics = collect_domain(&mut errors, inventory_domain::FABRICS, fabrics_result);

    Ok(RedfishReport {
        service_root,
        managers,
        systems,
        chassis,
        firmware,
        fabrics,
        errors,
    })
}

fn collect_domain<T>(
    errors: &mut Vec<DomainError>,
    domain: &str,
    result: Result<Vec<T>, String>,
) -> Vec<T> {
    match result {
        Ok(items) => items,
        Err(error) => {
            push_inventory_error(errors, domain, None, error);
            Vec::new()
        }
    }
}

fn description<T: ToString>(value: Option<&Option<T>>) -> Option<String> {
    value.and_then(Option::as_ref).map(ToString::to_string)
}

async fn collect_managers<B: Bmc>(root: &ServiceRoot<B>) -> Result<Vec<ManagerInventory>, String> {
    let Some(collection) = root
        .managers()
        .await
        .map_err(|err| format!("managers collection: {err}"))?
    else {
        return Ok(Vec::new());
    };

    let managers = collection
        .members()
        .await
        .map_err(|err| format!("manager members: {err}"))?;
    let mut out = Vec::with_capacity(managers.len());
    for manager in managers {
        let raw = manager.raw();
        out.push(ManagerInventory {
            name: raw.name.to_string(),
            description: description(raw.description.as_ref()),
            id: raw.id.to_string(),
            odata_id: Some(SanitizedUriReference::new(raw.odata_id.to_string())),
            uuid: raw.uuid.flatten().map(|uuid| uuid.to_string()),
            manager_type: raw.manager_type.and_then(redfish_enum_string),
            model: raw.model.clone().flatten(),
            firmware_version: raw.firmware_version.clone().flatten(),
            status: status_data(raw.status.as_ref()),
            links: manager_links(raw.links.as_ref()),
        });
    }
    Ok(out)
}

async fn collect_systems<B: Bmc>(
    root: &ServiceRoot<B>,
    errors: &mut Vec<DomainError>,
) -> Result<Vec<SystemInventory>, String> {
    let Some(collection) = root
        .systems()
        .await
        .map_err(|err| format!("systems collection: {err}"))?
    else {
        return Ok(Vec::new());
    };

    let systems = collection
        .members()
        .await
        .map_err(|err| format!("system members: {err}"))?;
    let mut out = Vec::with_capacity(systems.len());
    for system in systems {
        let raw = system.raw();
        let system_id = raw.id.to_string();
        out.push(SystemInventory {
            name: raw.name.to_string(),
            description: description(raw.description.as_ref()),
            id: system_id.clone(),
            odata_id: Some(SanitizedUriReference::new(raw.odata_id.to_string())),
            uuid: raw
                .uuid
                .flatten()
                .and_then(|uuid| InventoryUuid::parse(&uuid.to_string())),
            manufacturer: raw.manufacturer.clone().flatten(),
            model: raw.model.clone().flatten(),
            sku: raw.sku.clone().flatten(),
            serial_number: raw.serial_number.clone().flatten(),
            bios_version: raw.bios_version.clone().flatten(),
            host_name: raw.host_name.clone().flatten(),
            asset_tag: raw.asset_tag.clone().flatten(),
            power_state: raw.power_state.flatten().and_then(redfish_enum_string),
            status: status_data(raw.status.as_ref()),
            processor_summary: raw.processor_summary.as_ref().map(processor_summary_data),
            memory_summary: raw.memory_summary.as_ref().map(memory_summary_data),
            secure_boot_enabled: collect_secure_boot_enabled(&system, &system_id, errors).await,
            processors: collect_processors(&system, &system_id, errors).await,
            memory: collect_memory(&system, &system_id, errors).await,
            storage: collect_storage(&system, &system_id, errors).await,
            links: system_links(raw.links.as_ref()),
        });
    }
    Ok(out)
}

async fn collect_chassis<B: Bmc>(
    root: &ServiceRoot<B>,
    bmc: &B,
    errors: &mut Vec<DomainError>,
) -> Result<Vec<ChassisInventory>, String> {
    let Some(collection) = root
        .chassis()
        .await
        .map_err(|err| format!("chassis collection: {err}"))?
    else {
        return Ok(Vec::new());
    };

    let chassis = collection
        .members()
        .await
        .map_err(|err| format!("chassis members: {err}"))?;
    let mut out = Vec::with_capacity(chassis.len());
    for chassis in chassis {
        let raw = chassis.raw();
        let chassis_id = raw.id.to_string();
        out.push(ChassisInventory {
            name: raw.name.to_string(),
            description: description(raw.description.as_ref()),
            id: chassis_id.clone(),
            odata_id: Some(SanitizedUriReference::new(raw.odata_id.to_string())),
            chassis_type: redfish_enum_string(raw.chassis_type),
            manufacturer: raw.manufacturer.clone().flatten(),
            model: raw.model.clone().flatten(),
            sku: raw.sku.clone().flatten(),
            serial_number: raw.serial_number.clone().flatten(),
            part_number: raw.part_number.clone().flatten(),
            asset_tag: raw.asset_tag.clone().flatten(),
            power_state: raw.power_state.flatten().and_then(redfish_enum_string),
            status: status_data(raw.status.as_ref()),
            location: location_data(raw.location.as_ref()),
            pcie_devices: collect_pcie_devices(&chassis, &chassis_id, errors).await,
            power_supplies: collect_power_supplies(&chassis, bmc, &chassis_id, errors).await,
            links: chassis_links(raw.links.as_ref()),
        });
    }
    Ok(out)
}

async fn collect_secure_boot_enabled<B: Bmc>(
    system: &nv_redfish::computer_system::ComputerSystem<B>,
    system_id: &str,
    errors: &mut Vec<DomainError>,
) -> Option<bool> {
    let secure_boot = match system.secure_boot().await {
        Ok(secure_boot) => secure_boot?,
        Err(err) => {
            push_inventory_error(
                errors,
                inventory_domain::SYSTEMS_SECURE_BOOT,
                Some(system_id.to_string()),
                err,
            );
            return None;
        }
    };
    secure_boot.secure_boot_enable()
}

async fn collect_processors<B: Bmc>(
    system: &nv_redfish::computer_system::ComputerSystem<B>,
    system_id: &str,
    errors: &mut Vec<DomainError>,
) -> Vec<ProcessorInventory> {
    let processors = match system.processors().await {
        Ok(Some(processors)) => processors,
        Ok(None) => return Vec::new(),
        Err(err) => {
            push_inventory_error(
                errors,
                inventory_domain::SYSTEMS_PROCESSORS,
                Some(system_id.to_string()),
                err,
            );
            return Vec::new();
        }
    };

    let mut out = Vec::with_capacity(processors.len());
    for processor in processors {
        let raw = processor.raw();
        // Decode only processors that declare NVIDIA GPU OEM data. Other NVIDIA
        // processor types carry no GPU inventory, so their OEM data is not
        // parsed and cannot produce a processor domain error.
        let nvidia_gpu = if declares_nvidia_gpu(raw.oem.as_ref()) {
            match processor.oem_nvidia() {
                Ok(Some(NvidiaProcessor::Gpu(gpu))) => Some(nvidia_gpu_inventory(&gpu)),
                Ok(_) => None,
                Err(error) => {
                    push_inventory_error(
                        errors,
                        inventory_domain::SYSTEMS_PROCESSORS,
                        Some(raw.id.to_string()),
                        format!("NVIDIA GPU OEM data: {error}"),
                    );
                    None
                }
            }
        } else {
            None
        };

        out.push(ProcessorInventory {
            name: raw.name.to_string(),
            description: description(raw.description.as_ref()),
            id: raw.id.to_string(),
            odata_id: Some(SanitizedUriReference::new(raw.odata_id.to_string())),
            uuid: raw.uuid.flatten().map(|uuid| uuid.to_string()),
            socket: raw.socket.clone().flatten(),
            processor_type: raw.processor_type.flatten().and_then(redfish_enum_string),
            processor_architecture: raw
                .processor_architecture
                .clone()
                .flatten()
                .map(|architecture| architecture.to_string()),
            instruction_set: raw.instruction_set.clone().flatten(),
            manufacturer: raw.manufacturer.clone().flatten(),
            model: raw.model.clone().flatten(),
            serial_number: raw.serial_number.clone().flatten(),
            part_number: raw.part_number.clone().flatten(),
            location: location_data(raw.location.as_ref()),
            nvidia_gpu,
            max_speed_mhz: raw.max_speed_mhz.flatten(),
            total_cores: raw.total_cores.flatten(),
            total_threads: raw.total_threads.flatten(),
            status: status_data(raw.status.as_ref()),
            links: processor_links(raw.links.as_ref()),
        });
    }
    out
}

fn declares_nvidia_gpu(oem: Option<&nv_redfish::schema::resource::Oem>) -> bool {
    oem.and_then(|oem| nv_redfish::oem::oem_value(oem, nv_redfish::oem::nvidia::OEM_KEY))
        .is_some_and(|nvidia| nv_redfish::oem::declares(nvidia, "NvidiaProcessor", "NvidiaGPU"))
}

fn nvidia_gpu_inventory(gpu: &NvidiaGpu) -> NvidiaGpuInventory {
    let nvlink_topology = gpu
        .mnnv_link_topology
        .as_ref()
        .and_then(Option::as_ref)
        .map(|topology| NvlinkTopologyInventory {
            tray_serial_number: topology.tray_serial_number.clone().flatten(),
            tray_slot_number: topology.tray_slot_number.flatten(),
            tray_slot_index: topology.tray_slot_index.flatten(),
            chassis_serial_number: topology.chassis_serial_number.clone().flatten(),
            host_id: topology.host_id.flatten(),
            module_id: topology.module_id.flatten(),
            system_guid: topology.system_guid.clone().flatten(),
            ib_guid: topology.ibguid.clone().flatten(),
            lsb_ib_guid: topology.lsb_ibguid.clone().flatten(),
            peer_type: topology.peer_type.flatten().and_then(redfish_enum_string),
        });

    NvidiaGpuInventory {
        mig_mode_enabled: gpu.mig_mode_enabled.flatten(),
        default_boost_clock_speed_mhz: gpu.default_boost_clock_speed_mhz.flatten(),
        max_boost_clock_speed_mhz: gpu.max_boost_clock_speed_mhz.flatten(),
        cc_mode_enabled: gpu.cc_mode_enabled.flatten(),
        cc_dev_mode_enabled: gpu.cc_dev_mode_enabled.flatten(),
        protected_pcie_mode_enabled: gpu.ppcie_mode_enabled.flatten(),
        egm_mode_enabled: gpu.egm_mode_enabled.flatten(),
        total_nv_links: gpu.total_number_nv_links.flatten(),
        nvlink_topology,
    }
}

async fn collect_pcie_devices<B: Bmc>(
    chassis: &nv_redfish::chassis::Chassis<B>,
    chassis_id: &str,
    errors: &mut Vec<DomainError>,
) -> Vec<PcieDeviceInventory> {
    let devices = match chassis.pcie_devices().await {
        Ok(Some(collection)) => collection.members().await.map_err(|err| err.to_string()),
        Ok(None) => return Vec::new(),
        Err(err) => Err(err.to_string()),
    };
    let devices = match devices {
        Ok(devices) => devices,
        Err(err) => {
            push_inventory_error(
                errors,
                inventory_domain::CHASSIS_PCIE_DEVICES,
                Some(chassis_id.to_string()),
                err,
            );
            return Vec::new();
        }
    };

    let mut out = Vec::with_capacity(devices.len());
    for device in devices {
        let raw = device.raw();
        out.push(PcieDeviceInventory {
            name: raw.name.to_string(),
            description: description(raw.description.as_ref()),
            id: raw.id.to_string(),
            odata_id: Some(SanitizedUriReference::new(raw.odata_id.to_string())),
            uuid: raw.uuid.flatten().map(|uuid| uuid.to_string()),
            device_type: raw.device_type.and_then(redfish_enum_string),
            manufacturer: raw.manufacturer.clone().flatten(),
            model: raw.model.clone().flatten(),
            sku: raw.sku.clone().flatten(),
            serial_number: raw.serial_number.clone().flatten(),
            part_number: raw.part_number.clone().flatten(),
            firmware_version: raw.firmware_version.clone().flatten(),
            status: status_data(raw.status.as_ref()),
            links: pcie_device_links(raw.links.as_ref()),
        });
    }
    out
}

async fn collect_firmware<B: Bmc>(root: &ServiceRoot<B>) -> Result<Vec<FirmwareInventory>, String> {
    let Some(update_service) = root
        .update_service()
        .await
        .map_err(|err| format!("update service: {err}"))?
    else {
        return Ok(Vec::new());
    };
    let inventories = update_service
        .firmware_inventories()
        .await
        .map_err(|err| format!("firmware inventories: {err}"))?
        .unwrap_or_default();

    Ok(inventories
        .into_iter()
        .map(|inventory| {
            let raw = inventory.raw();
            FirmwareInventory {
                description: description(raw.description.as_ref()),
                related_item: raw.related_item.as_deref().map(link_targets),
                service_id: "FirmwareInventory".to_string(),
                id: raw.id.to_string(),
                odata_id: Some(SanitizedUriReference::new(raw.odata_id.to_string())),
                name: raw.name.to_string(),
                version: raw.version.clone().flatten(),
                release_date: raw.release_date.flatten().map(|date| date.to_string()),
                status: status_data(raw.status.as_ref()),
                links: firmware_links(raw.links.as_ref()),
            }
        })
        .collect())
}

fn processor_summary_data(
    summary: &nv_redfish::schema::computer_system::ProcessorSummary,
) -> ProcessorSummary {
    ProcessorSummary {
        count: summary.count.flatten(),
        core_count: summary.core_count.flatten(),
        model: summary.model.clone().flatten(),
        logical_processor_count: summary.logical_processor_count.flatten(),
        threading_enabled: summary.threading_enabled,
        status: status_data(summary.status.as_ref()),
    }
}

fn memory_summary_data(
    summary: &nv_redfish::schema::computer_system::MemorySummary,
) -> MemorySummary {
    MemorySummary {
        total_system_memory_gib: summary.total_system_memory_gi_b.flatten(),
        total_system_persistent_memory_gib: summary.total_system_persistent_memory_gi_b.flatten(),
        memory_mirroring: summary
            .memory_mirroring
            .flatten()
            .and_then(redfish_enum_string),
        status: status_data(summary.status.as_ref()),
    }
}

/// Fetches each fabric and switch independently: one unreadable member is
/// recorded as a scoped domain error and does not discard its siblings.
async fn collect_fabrics<B: Bmc>(
    root: &ServiceRoot<B>,
    errors: &mut Vec<DomainError>,
) -> Result<Vec<FabricInventory>, String> {
    let Some(collection) = root
        .fabrics()
        .await
        .map_err(|err| format!("fabrics collection: {err}"))?
    else {
        return Ok(Vec::new());
    };

    let links = collection.member_links();
    let mut out = Vec::with_capacity(links.len());
    for link in links {
        let fabric: Fabric<B> = match link.upgrade().await {
            Ok(fabric) => fabric,
            Err(err) => {
                push_inventory_error(
                    errors,
                    inventory_domain::FABRICS,
                    link.odata_id().last_segment().map(ToString::to_string),
                    err,
                );
                continue;
            }
        };
        let raw = fabric.raw();
        let fabric_id = raw.id.to_string();
        out.push(FabricInventory {
            name: raw.name.to_string(),
            description: description(raw.description.as_ref()),
            id: fabric_id.clone(),
            odata_id: Some(SanitizedUriReference::new(raw.odata_id.to_string())),
            uuid: raw.uuid.flatten().map(|uuid| uuid.to_string()),
            fabric_type: raw.fabric_type.flatten().and_then(redfish_enum_string),
            status: status_data(raw.status.as_ref()),
            switches: collect_switches(&fabric, &fabric_id, errors).await,
            links: fabric_links(raw.links.as_ref()),
        });
    }
    Ok(out)
}

async fn collect_switches<B: Bmc>(
    fabric: &Fabric<B>,
    fabric_id: &str,
    errors: &mut Vec<DomainError>,
) -> Vec<SwitchInventory> {
    let collection = match fabric.switches().await {
        Ok(Some(collection)) => collection,
        Ok(None) => return Vec::new(),
        Err(err) => {
            push_inventory_error(
                errors,
                inventory_domain::FABRICS_SWITCHES,
                Some(fabric_id.to_string()),
                err,
            );
            return Vec::new();
        }
    };

    let links = collection.member_links();
    let mut out = Vec::with_capacity(links.len());
    for link in links {
        let switch: Switch<B> = match link.upgrade().await {
            Ok(switch) => switch,
            Err(err) => {
                push_inventory_error(
                    errors,
                    inventory_domain::FABRICS_SWITCHES,
                    link.odata_id().last_segment().map(ToString::to_string),
                    err,
                );
                continue;
            }
        };
        let raw = switch.raw();
        let nvidia_switch = match switch.oem_nvidia() {
            Ok(nvidia) => nvidia.map(|nvidia| nvidia_switch_inventory(&nvidia.raw())),
            Err(err) => {
                push_inventory_error(
                    errors,
                    inventory_domain::FABRICS_SWITCHES,
                    Some(raw.id.to_string()),
                    format!("NVIDIA switch OEM data: {err}"),
                );
                None
            }
        };
        out.push(SwitchInventory {
            name: raw.name.to_string(),
            description: description(raw.description.as_ref()),
            id: raw.id.to_string(),
            odata_id: Some(SanitizedUriReference::new(raw.odata_id.to_string())),
            uuid: raw.uuid.flatten().map(|uuid| uuid.to_string()),
            switch_type: raw.switch_type.flatten().and_then(redfish_enum_string),
            manufacturer: raw.manufacturer.clone().flatten(),
            model: raw.model.clone().flatten(),
            sku: raw.sku.clone().flatten(),
            serial_number: raw.serial_number.clone().flatten(),
            part_number: raw.part_number.clone().flatten(),
            firmware_version: raw.firmware_version.clone().flatten(),
            location: location_data(raw.location.as_ref()),
            nvidia_switch,
            status: status_data(raw.status.as_ref()),
            links: switch_links(raw.links.as_ref()),
        });
    }
    out
}

fn nvidia_switch_inventory(switch: &NvidiaSwitchSchema) -> NvidiaSwitchInventory {
    NvidiaSwitchInventory {
        device_id: switch.device_id.clone().flatten(),
        vendor_id: switch.vendor_id.clone().flatten(),
        pcie_reference_clock_enabled: switch.pcie_reference_clock_enabled.flatten(),
        protected_pcie_mode_enabled: switch.ppcie_mode_enabled.flatten(),
        switch_isolation_mode: switch
            .switch_isolation_mode
            .flatten()
            .and_then(redfish_enum_string),
    }
}

fn push_inventory_error(
    errors: &mut Vec<DomainError>,
    domain: &str,
    resource: Option<String>,
    error: impl std::fmt::Display,
) {
    errors.push(DomainError {
        domain: domain.to_string(),
        resource: resource.map(SanitizedUriReference::new),
        message: SanitizedErrorMessage::new(error.to_string()),
    });
}

async fn collect_memory<B: Bmc>(
    system: &nv_redfish::computer_system::ComputerSystem<B>,
    system_id: &str,
    errors: &mut Vec<DomainError>,
) -> Vec<MemoryInventory> {
    let modules = match system.memory_modules().await {
        Ok(modules) => modules.unwrap_or_default(),
        Err(err) => {
            push_inventory_error(
                errors,
                inventory_domain::SYSTEMS_MEMORY,
                Some(system_id.to_string()),
                err,
            );
            return Vec::new();
        }
    };
    modules
        .into_iter()
        .map(|module| {
            let raw = module.raw();
            MemoryInventory {
                id: raw.id.to_string(),
                system_id: system_id.to_string(),
                odata_id: Some(raw.odata_id.to_string().into()),
                memory_device_type: raw
                    .memory_device_type
                    .flatten()
                    .and_then(redfish_enum_string),
                capacity_mib: raw.capacity_mi_b.flatten(),
                serial_number: raw.serial_number.clone().flatten(),
                part_number: raw.part_number.clone().flatten(),
                manufacturer: raw.manufacturer.clone().flatten(),
                device_locator: raw.device_locator.clone().flatten(),
                location: location_data(raw.location.as_ref()),
                status: status_data(raw.status.as_ref()),
            }
        })
        .collect()
}

async fn collect_storage<B: Bmc>(
    system: &nv_redfish::computer_system::ComputerSystem<B>,
    system_id: &str,
    errors: &mut Vec<DomainError>,
) -> Vec<StorageInventory> {
    let controllers = match system.storage_controllers().await {
        Ok(controllers) => controllers.unwrap_or_default(),
        Err(err) => {
            push_inventory_error(
                errors,
                inventory_domain::SYSTEMS_STORAGE,
                Some(system_id.to_string()),
                err,
            );
            return Vec::new();
        }
    };
    let mut out = Vec::with_capacity(controllers.len());
    for controller in controllers {
        let raw = controller.raw();
        let storage_id = raw.id.to_string();
        let drives = match controller.drives().await {
            Ok(drives) => drives.unwrap_or_default(),
            Err(err) => {
                // Preserve the stored system's entire storage section if any drive
                // list is incomplete; other systems and inventory domains still update.
                push_inventory_error(
                    errors,
                    inventory_domain::SYSTEMS_STORAGE,
                    Some(system_id.to_string()),
                    err,
                );
                Vec::new()
            }
        };
        let drives = drives
            .into_iter()
            .map(|drive| {
                let raw = drive.raw();
                DriveInventory {
                    id: raw.id.to_string(),
                    system_id: system_id.to_string(),
                    storage_id: storage_id.clone(),
                    odata_id: Some(raw.odata_id.to_string().into()),
                    model: raw.model.clone().flatten(),
                    serial_number: raw.serial_number.clone().flatten(),
                    capacity_bytes: raw.capacity_bytes.flatten(),
                    protocol: raw.protocol.flatten().and_then(redfish_enum_string),
                    media_type: raw.media_type.flatten().and_then(redfish_enum_string),
                    revision: raw.revision.clone().flatten(),
                    status: status_data(raw.status.as_ref()),
                }
            })
            .collect();
        out.push(StorageInventory {
            id: storage_id,
            system_id: system_id.to_string(),
            odata_id: Some(raw.odata_id.to_string().into()),
            drives,
        });
    }
    out
}

async fn collect_power_supplies<B: Bmc>(
    chassis: &nv_redfish::chassis::Chassis<B>,
    bmc: &B,
    chassis_id: &str,
    errors: &mut Vec<DomainError>,
) -> Vec<PowerSupplyInventory> {
    let supplies = match chassis.power_supplies().await {
        Ok(supplies) => supplies,
        Err(err) => {
            push_inventory_error(
                errors,
                inventory_domain::CHASSIS_POWER_SUPPLIES,
                Some(chassis_id.to_string()),
                err,
            );
            return Vec::new();
        }
    };
    if supplies.is_empty() && chassis.raw().power_subsystem.is_none() {
        let power = match chassis.power().await {
            Ok(Some(power)) => power,
            Ok(None) => return Vec::new(),
            Err(err) => {
                push_inventory_error(
                    errors,
                    inventory_domain::CHASSIS_POWER_SUPPLIES,
                    Some(chassis_id.to_string()),
                    err,
                );
                return Vec::new();
            }
        };
        let raw_power = power.raw();
        let mut out = Vec::new();
        for link in raw_power.power_supplies.as_deref().unwrap_or_default() {
            let raw = match link.get(bmc).await {
                Ok(raw) => raw,
                Err(err) => {
                    push_inventory_error(
                        errors,
                        inventory_domain::CHASSIS_POWER_SUPPLIES,
                        Some(chassis_id.to_string()),
                        err,
                    );
                    continue;
                }
            };
            out.push(PowerSupplyInventory {
                id: raw.member_id.to_string(),
                chassis_id: chassis_id.to_string(),
                odata_id: Some(raw.odata_id.to_string().into()),
                model: raw.model.clone().flatten(),
                serial_number: raw.serial_number.clone().flatten(),
                part_number: raw.part_number.clone().flatten(),
                firmware_version: raw.firmware_version.clone().flatten(),
                power_capacity_watts: raw.power_capacity_watts.flatten(),
                status: status_data(raw.status.as_ref()),
            });
        }
        return out;
    }

    supplies
        .into_iter()
        .map(|supply| {
            let raw = supply.raw();
            PowerSupplyInventory {
                id: raw.id.to_string(),
                chassis_id: chassis_id.to_string(),
                odata_id: Some(raw.odata_id.to_string().into()),
                model: raw.model.clone().flatten(),
                serial_number: raw.serial_number.clone().flatten(),
                part_number: raw.part_number.clone().flatten(),
                firmware_version: raw.firmware_version.clone().flatten(),
                power_capacity_watts: raw.power_capacity_watts.flatten(),
                status: status_data(raw.status.as_ref()),
            }
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeMap;
    use std::convert::Infallible;
    use std::sync::Arc;

    use super::super::links::link_reference;
    use super::*;
    use nv_redfish_bmc_mock::Expect;

    struct MockBmcResponses {
        manager: &'static str,
        system: &'static str,
        processor: Option<&'static str>,
        secure_boot: Option<&'static str>,
        pcie_collection: &'static str,
        pcie_device: Option<&'static str>,
        firmware: &'static str,
    }

    impl Default for MockBmcResponses {
        fn default() -> Self {
            Self {
                manager: MANAGER,
                system: SYSTEM,
                processor: None,
                secure_boot: None,
                pcie_collection: PCIE_DEVICES_COLLECTION,
                pcie_device: Some(PCIE_DEVICE),
                firmware: FIRMWARE,
            }
        }
    }

    async fn collect_mock_inventory(fixture: MockBmcResponses) -> RedfishReport {
        type MockBmc = nv_redfish_bmc_mock::Bmc<Infallible>;

        let bmc = Arc::new(MockBmc::default());
        bmc.expect(Expect::get("/redfish/v1", SERVICE_ROOT));
        bmc.expect(Expect::get("/redfish/v1/Managers", MANAGERS_COLLECTION));
        bmc.expect(Expect::get("/redfish/v1/Managers/BMC_0", fixture.manager));
        bmc.expect(Expect::get("/redfish/v1/Systems", SYSTEMS_COLLECTION));
        bmc.expect(Expect::get("/redfish/v1/Systems/System", fixture.system));
        if let Some(processor) = fixture.processor {
            bmc.expect(Expect::get(
                "/redfish/v1/Systems/System/Processors",
                PROCESSORS_COLLECTION,
            ));
            bmc.expect(Expect::get(
                "/redfish/v1/Systems/System/Processors/CPU1",
                processor,
            ));
        }
        if let Some(secure_boot) = fixture.secure_boot {
            bmc.expect(Expect::get(
                "/redfish/v1/Systems/System/SecureBoot",
                secure_boot,
            ));
        }
        bmc.expect(Expect::get("/redfish/v1/Chassis", CHASSIS_COLLECTION));
        bmc.expect(Expect::get("/redfish/v1/Chassis/HGX_GPU_SXM_0", CHASSIS));
        bmc.expect(Expect::get(
            "/redfish/v1/Chassis/HGX_GPU_SXM_0/PCIeDevices",
            fixture.pcie_collection,
        ));
        if let Some(pcie_device) = fixture.pcie_device {
            bmc.expect(Expect::get(
                "/redfish/v1/Chassis/HGX_GPU_SXM_0/PCIeDevices/GPU1",
                pcie_device,
            ));
        }
        bmc.expect(Expect::get("/redfish/v1/UpdateService", UPDATE_SERVICE));
        bmc.expect(Expect::get(
            "/redfish/v1/UpdateService/FirmwareInventory",
            FIRMWARE_COLLECTION,
        ));
        bmc.expect(Expect::get(
            "/redfish/v1/UpdateService/FirmwareInventory/BMC",
            fixture.firmware,
        ));

        let root = ServiceRoot::new(bmc.clone())
            .await
            .expect("service root wrapper");
        collect_report_from_root(&root, bmc.as_ref())
            .await
            .expect("report")
    }

    #[test]
    fn summaries_preserve_status_conditions_and_partial_fields() {
        let processor = serde_json::from_value(serde_json::json!({
            "Count": 2, "CoreCount": 144, "LogicalProcessorCount": 288, "ThreadingEnabled": true,
            "Status": {"Health": "Critical", "Conditions": [{"MessageId": "ResourceEvent.1.0.ResourceErrorsDetected", "Severity": "Critical"}]}
        })).unwrap();
        let mapped = processor_summary_data(&processor);
        assert_eq!(mapped.count, Some(2));
        assert_eq!(mapped.logical_processor_count, Some(288));
        assert_eq!(mapped.threading_enabled, Some(true));
        assert_eq!(
            mapped.status.as_ref().unwrap().health.as_deref(),
            Some("Critical")
        );
        assert_eq!(
            mapped
                .status
                .as_ref()
                .unwrap()
                .conditions
                .as_ref()
                .unwrap()
                .len(),
            1
        );
        let memory = serde_json::from_value(serde_json::json!({
            "TotalSystemMemoryGiB": 480, "TotalSystemPersistentMemoryGiB": 32, "MemoryMirroring": "DIMM",
            "Status": {"Health": "Warning", "Conditions": []}
        })).unwrap();
        let mapped = memory_summary_data(&memory);
        assert_eq!(mapped.total_system_persistent_memory_gib, Some(32.0));
        assert_eq!(mapped.memory_mirroring.as_deref(), Some("DIMM"));
        assert_eq!(
            mapped
                .status
                .as_ref()
                .unwrap()
                .conditions
                .as_ref()
                .unwrap()
                .len(),
            0
        );
        let empty = serde_json::from_value(serde_json::json!({})).unwrap();
        let mapped = processor_summary_data(&empty);
        assert_eq!(mapped.count, None);
        assert!(mapped.status.is_none());
    }

    #[tokio::test]
    async fn non_gpu_nvidia_processor_oem_has_no_gpu_inventory() {
        let report = collect_mock_inventory(MockBmcResponses {
            system: SYSTEM_WITH_PROCESSORS,
            processor: Some(PROCESSOR_NON_GPU_OEM),
            ..Default::default()
        })
        .await;

        let processor = &report.systems[0].processors[0];
        assert_eq!(processor.id, "CPU1");
        assert!(processor.nvidia_gpu.is_none());
        assert!(report.errors.is_empty());
    }

    #[tokio::test]
    async fn malformed_non_gpu_nvidia_oem_is_not_decoded() {
        let report = collect_mock_inventory(MockBmcResponses {
            system: SYSTEM_WITH_PROCESSORS,
            processor: Some(PROCESSOR_MALFORMED_LPU_OEM),
            ..Default::default()
        })
        .await;

        let processor = &report.systems[0].processors[0];
        assert_eq!(processor.id, "CPU1");
        assert!(processor.nvidia_gpu.is_none());
        assert!(report.errors.is_empty(), "{:?}", report.errors);
    }

    #[tokio::test]
    async fn malformed_gpu_oem_keeps_processor_and_reports_domain_error() {
        let report = collect_mock_inventory(MockBmcResponses {
            system: SYSTEM_WITH_PROCESSORS,
            processor: Some(PROCESSOR_MALFORMED_GPU_OEM),
            ..Default::default()
        })
        .await;

        let processor = &report.systems[0].processors[0];
        assert_eq!(processor.id, "CPU1");
        assert!(processor.nvidia_gpu.is_none());
        assert_eq!(report.errors.len(), 1);
        assert_eq!(
            report.errors[0].domain,
            inventory_domain::SYSTEMS_PROCESSORS
        );
        assert_eq!(report.errors[0].resource.as_deref(), Some("CPU1"));
        assert!(report.errors[0].message.contains("NVIDIA GPU OEM data"));
    }

    #[test]
    fn source_sanitizes_domain_errors() {
        let url = "https://user:secret@bmc.example.com/redfish/v1/Chassis/A?token=secret#part";
        let mut errors = Vec::new();

        push_inventory_error(
            &mut errors,
            inventory_domain::CHASSIS,
            Some(url.to_string()),
            format!("GET {url} failed"),
        );

        assert_eq!(errors.len(), 1);
        let error = &errors[0];
        assert_eq!(error.domain, inventory_domain::CHASSIS);
        assert_eq!(error.message.as_ref(), "GET [URL_REDACTED] failed");
        assert_eq!(
            error.resource.as_ref().map(AsRef::as_ref),
            Some("/redfish/v1/Chassis/A#part")
        );
    }

    #[tokio::test]
    async fn collect_report_from_mock_bmc_traverses_inventory() {
        let report = collect_mock_inventory(MockBmcResponses {
            system: SYSTEM_WITH_PROCESSORS,
            processor: Some(PROCESSOR),
            ..Default::default()
        })
        .await;

        assert_eq!(
            report.service_root.redfish_version.as_deref(),
            Some("1.15.1")
        );
        assert_eq!(report.service_root.uuid.as_deref(), Some(SERVICE_UUID));
        assert_eq!(report.service_root.vendor.as_deref(), Some("AMI"));
        assert_eq!(report.service_root.product.as_deref(), Some("Mock BMC"));

        assert_eq!(report.managers.len(), 1);
        assert_eq!(report.managers[0].id, "BMC_0");
        assert_eq!(report.managers[0].name, "BMC");
        assert_eq!(
            report.managers[0].description.as_deref(),
            Some("  Baseboard management controller  ")
        );
        assert_eq!(
            report.managers[0].firmware_version.as_deref(),
            Some("3.19.00")
        );
        assert_eq!(
            report.managers[0].links,
            BTreeMap::from([(
                "ManagerForServers".to_string(),
                vec![link_reference(
                    "/redfish/v1/Systems/System#item".to_string()
                )]
            )])
        );

        assert_eq!(report.systems.len(), 1);
        assert_eq!(report.systems[0].id, "DGX");
        assert_eq!(
            report.systems[0]
                .uuid
                .as_ref()
                .map(ToString::to_string)
                .as_deref(),
            Some(SYSTEM_UUID)
        );
        assert_eq!(report.systems[0].manufacturer.as_deref(), Some("NVIDIA"));
        assert_eq!(report.systems[0].model.as_deref(), Some("GB200"));
        assert_eq!(report.systems[0].sku.as_deref(), Some("SKU-GB200"));
        assert_eq!(report.systems[0].serial_number.as_deref(), Some("SN123"));
        assert_eq!(report.systems[0].bios_version.as_deref(), Some("3A21"));
        assert_eq!(
            report.systems[0].host_name.as_deref(),
            Some("node-a.example.com")
        );
        assert_eq!(
            report.systems[0].asset_tag.as_deref(),
            Some("rack-12-tray-3")
        );
        assert_eq!(report.systems[0].power_state.as_deref(), Some("On"));
        assert_eq!(
            report.systems[0].processor_summary.as_ref().unwrap().count,
            Some(1)
        );
        assert_eq!(
            report.systems[0]
                .processor_summary
                .as_ref()
                .unwrap()
                .core_count,
            Some(72)
        );
        assert_eq!(
            report.systems[0]
                .processor_summary
                .as_ref()
                .unwrap()
                .model
                .as_deref(),
            Some("Grace")
        );
        assert_eq!(
            report.systems[0]
                .memory_summary
                .as_ref()
                .unwrap()
                .total_system_memory_gib,
            Some(480.0)
        );
        assert_eq!(report.systems[0].secure_boot_enabled, None);
        assert_eq!(report.systems[0].processors.len(), 1);
        let processor = &report.systems[0].processors[0];
        assert_eq!(processor.id, "CPU1");
        assert_eq!(
            processor.odata_id.as_deref(),
            Some("/redfish/v1/Systems/System/Processors/CPU1")
        );
        assert_eq!(
            processor.uuid.as_deref(),
            Some("22222222-3333-4444-5555-666666666666")
        );
        assert_eq!(processor.socket.as_deref(), Some("CPU Socket 1"));
        assert_eq!(processor.processor_type.as_deref(), Some("CPU"));
        assert_eq!(processor.processor_architecture.as_deref(), Some("ARM"));
        assert_eq!(processor.instruction_set.as_deref(), Some("ARM-A64"));
        assert_eq!(processor.manufacturer.as_deref(), Some("NVIDIA"));
        assert_eq!(processor.model.as_deref(), Some("Grace"));
        assert_eq!(processor.serial_number.as_deref(), Some("CPU-SN123"));
        assert_eq!(processor.part_number.as_deref(), Some("CPU-PN123"));
        let location = processor.location.as_ref().expect("processor location");
        assert_eq!(
            location.part_location_context.as_deref(),
            Some("HGX_Chassis_0/ProcessorModule_0")
        );
        let physical_location = location
            .part_location
            .as_ref()
            .expect("processor part location");
        assert_eq!(
            physical_location.service_label.as_deref(),
            Some("GPU_SXM_1")
        );
        assert_eq!(physical_location.location_type.as_deref(), Some("Slot"));
        assert_eq!(physical_location.location_ordinal_value, Some(1));
        assert_eq!(
            physical_location.orientation.as_deref(),
            Some("LeftToRight")
        );
        let nvidia_gpu = processor.nvidia_gpu.as_ref().expect("NVIDIA GPU data");
        assert_eq!(nvidia_gpu.mig_mode_enabled, Some(false));
        assert_eq!(nvidia_gpu.default_boost_clock_speed_mhz, Some(1200));
        assert_eq!(nvidia_gpu.max_boost_clock_speed_mhz, Some(1800));
        assert_eq!(nvidia_gpu.cc_mode_enabled, Some(true));
        assert_eq!(nvidia_gpu.total_nv_links, Some(18));
        assert_eq!(
            nvidia_gpu
                .nvlink_topology
                .as_ref()
                .and_then(|topology| topology.tray_slot_index),
            Some(2)
        );
        let nvlink_topology = nvidia_gpu
            .nvlink_topology
            .as_ref()
            .expect("NVLink topology");
        assert_eq!(
            nvlink_topology.chassis_serial_number.as_deref(),
            Some("chassis-serial")
        );
        assert_eq!(nvlink_topology.host_id, Some(17));
        assert_eq!(nvlink_topology.ib_guid.as_deref(), Some("ib-guid"));
        assert_eq!(nvlink_topology.peer_type.as_deref(), Some("Direct"));
        assert_eq!(processor.max_speed_mhz, Some(3600));
        assert_eq!(processor.total_cores, Some(72));
        assert_eq!(processor.total_threads, Some(72));
        let processor_status = processor.status.as_ref().expect("processor status");
        assert_eq!(processor_status.state.as_deref(), Some("Enabled"));
        assert_eq!(processor_status.health.as_deref(), Some("OK"));
        assert_eq!(processor_status.health_rollup.as_deref(), Some("OK"));
        let system_status = report.systems[0].status.as_ref().expect("system status");
        assert_eq!(system_status.state.as_deref(), Some("Enabled"));
        assert_eq!(system_status.health.as_deref(), Some("OK"));
        assert_eq!(system_status.health_rollup.as_deref(), Some("Warning"));
        assert_eq!(
            report.systems[0].links,
            BTreeMap::from([
                (
                    "Chassis".into(),
                    vec![link_reference(
                        "/redfish/v1/Chassis/HGX_GPU_SXM_0".to_string()
                    )]
                ),
                (
                    "ManagedBy".into(),
                    vec![link_reference("/redfish/v1/Managers/BMC_0".to_string())]
                )
            ])
        );
        assert_eq!(
            processor.links,
            BTreeMap::from([(
                "Chassis".to_string(),
                vec![link_reference(
                    "/redfish/v1/Chassis/HGX_GPU_SXM_0".to_string()
                )]
            )])
        );

        assert_eq!(report.chassis.len(), 1);
        assert_eq!(report.chassis[0].id, "HGX_GPU_SXM_0");
        assert_eq!(report.chassis[0].chassis_type.as_deref(), Some("Module"));
        assert_eq!(report.chassis[0].manufacturer.as_deref(), Some("NVIDIA"));
        assert_eq!(report.chassis[0].model.as_deref(), Some("BMC"));
        assert_eq!(report.chassis[0].serial_number.as_deref(), Some("CH123"));
        assert_eq!(report.chassis[0].part_number.as_deref(), Some("PN123"));
        let chassis_status = report.chassis[0].status.as_ref().expect("chassis status");
        assert_eq!(chassis_status.health.as_deref(), Some("Critical"));
        let conditions = chassis_status
            .conditions
            .as_ref()
            .expect("chassis conditions");
        assert_eq!(conditions.len(), 1);
        let condition = &conditions[0];
        assert_eq!(
            condition.message_id,
            "ResourceEvent.1.0.ResourceErrorsDetected"
        );
        assert_eq!(
            condition
                .message_args
                .as_ref()
                .map(|args| args.iter().map(AsRef::as_ref).collect::<Vec<_>>()),
            Some(vec!["CPU_1 Firmware", "EC_STRAP_MISMATCH"])
        );
        assert_eq!(
            condition.message.as_deref(),
            Some(
                "The resource property CPU_1 Firmware has detected errors of type 'EC_STRAP_MISMATCH'."
            )
        );
        assert_eq!(condition.severity.as_deref(), Some("Critical"));
        assert_eq!(
            condition.resolution.as_deref(),
            Some("Reset the component; collect logs and reflash if the problem remains.")
        );
        assert_eq!(condition.timestamp.as_deref(), Some("2026-08-25T20:51:47Z"));
        assert_eq!(
            condition
                .origin_of_condition
                .as_ref()
                .map(|origin| origin.odata_id.as_ref()),
            Some("/redfish/v1/Chassis/HGX_CPU_1")
        );
        let location = report.chassis[0]
            .location
            .as_ref()
            .expect("chassis location");
        assert_eq!(
            location
                .part_location
                .as_ref()
                .unwrap()
                .service_label
                .as_deref(),
            Some("Bay 1")
        );
        assert_eq!(
            location.placement.as_ref().unwrap().rack.as_deref(),
            Some("R12")
        );
        assert_eq!(
            location.placement.as_ref().unwrap().row.as_deref(),
            Some("A")
        );
        assert_eq!(location.placement.as_ref().unwrap().rack_offset, Some(7));
        assert_eq!(report.chassis[0].links.len(), 2);

        assert_eq!(report.chassis[0].pcie_devices.len(), 1);
        assert_eq!(report.chassis[0].pcie_devices[0].id, "GPU1");
        assert_eq!(
            report.chassis[0].pcie_devices[0].serial_number.as_deref(),
            Some("GPU-SN456")
        );
        assert_eq!(
            report.chassis[0].pcie_devices[0]
                .status
                .as_ref()
                .and_then(|status| status.conditions.as_ref())
                .map(Vec::len),
            Some(0)
        );
        assert_eq!(report.chassis[0].pcie_devices[0].links.len(), 1);

        assert_eq!(report.firmware.len(), 1);
        assert_eq!(report.firmware[0].service_id, "FirmwareInventory");
        assert_eq!(report.firmware[0].id, "BMC");
        assert_eq!(report.firmware[0].name, "BMC");
        assert_eq!(report.firmware[0].version.as_deref(), Some("3.19.00"));
        assert_eq!(
            report.firmware[0].release_date.as_deref(),
            Some("2026-01-01T00:00:00Z")
        );
        let firmware_status = report.firmware[0].status.as_ref().expect("firmware status");
        assert_eq!(firmware_status.state.as_deref(), Some("StandbyOffline"));
        assert_eq!(firmware_status.health.as_deref(), Some("Critical"));
        assert_eq!(firmware_status.health_rollup.as_deref(), Some("Warning"));
        assert_eq!(report.firmware[0].links.len(), 1);
        assert_eq!(
            report.firmware[0].related_item,
            Some(vec![link_reference(
                "/redfish/v1/Managers/BMC_0#/Firmware".to_string()
            )])
        );
    }

    #[tokio::test]
    async fn secure_boot_collection_failure_records_system_id_resource() {
        let report = collect_mock_inventory(MockBmcResponses {
            system: SYSTEM_WITH_SECURE_BOOT,
            secure_boot: Some(MALFORMED_SECURE_BOOT),
            ..Default::default()
        })
        .await;

        assert_eq!(report.systems.len(), 1);
        assert_eq!(report.systems[0].id, "DGX");
        assert_eq!(report.systems[0].secure_boot_enabled, None);
        assert_eq!(report.errors.len(), 1);
        assert_eq!(
            report.errors[0].domain,
            inventory_domain::SYSTEMS_SECURE_BOOT
        );
        assert_eq!(report.errors[0].resource.as_deref(), Some("DGX"));
        assert!(!report.errors[0].message.is_empty());
    }

    #[tokio::test]
    async fn pcie_devices_collection_failure_records_chassis_id_resource() {
        let report = collect_mock_inventory(MockBmcResponses {
            pcie_collection: MALFORMED_PCIE_DEVICES_COLLECTION,
            pcie_device: None,
            ..Default::default()
        })
        .await;

        assert_eq!(report.chassis.len(), 1);
        assert_eq!(report.chassis[0].id, "HGX_GPU_SXM_0");
        assert!(report.chassis[0].pcie_devices.is_empty());
        assert_eq!(report.errors.len(), 1);
        assert_eq!(
            report.errors[0].domain,
            inventory_domain::CHASSIS_PCIE_DEVICES
        );
        assert_eq!(report.errors[0].resource.as_deref(), Some("HGX_GPU_SXM_0"));
        assert!(!report.errors[0].message.is_empty());
    }

    #[tokio::test]
    async fn manager_collection_failure_is_partial_and_keeps_other_inventory() {
        let report = collect_mock_inventory(MockBmcResponses {
            manager: MALFORMED_MANAGER,
            ..Default::default()
        })
        .await;

        assert!(report.managers.is_empty());
        assert_eq!(report.systems.len(), 1);
        assert_eq!(report.chassis.len(), 1);
        assert_eq!(report.chassis[0].pcie_devices.len(), 1);
        assert_eq!(report.firmware.len(), 1);
        assert_eq!(report.errors.len(), 1);
        assert_eq!(report.errors[0].domain, "managers");
        assert!(report.errors[0].resource.is_none());
        assert!(!report.errors[0].message.is_empty());
    }

    #[tokio::test]
    async fn firmware_collection_failure_is_partial_and_keeps_other_inventory() {
        let report = collect_mock_inventory(MockBmcResponses {
            firmware: MALFORMED_FIRMWARE,
            ..Default::default()
        })
        .await;

        assert_eq!(report.managers.len(), 1);
        assert_eq!(report.systems.len(), 1);
        assert_eq!(report.chassis.len(), 1);
        assert_eq!(report.chassis[0].pcie_devices.len(), 1);
        assert!(report.firmware.is_empty());
        assert_eq!(report.errors.len(), 1);
        assert_eq!(report.errors[0].domain, "firmware");
        assert!(report.errors[0].resource.is_none());
        assert!(!report.errors[0].message.is_empty());
    }

    const SERVICE_UUID: &str = "870155dd-d474-aca8-11ee-6e809af87c0c";
    const SYSTEM_UUID: &str = "9aee71e4-6e80-11ee-a8ac-74d4dd550187";

    async fn collect_mock_fabrics(
        responses: &[(&'static str, &'static str)],
    ) -> (Vec<FabricInventory>, Vec<DomainError>) {
        type MockBmc = nv_redfish_bmc_mock::Bmc<Infallible>;

        let bmc = Arc::new(MockBmc::default());
        bmc.expect(Expect::get("/redfish/v1", FABRIC_SERVICE_ROOT));
        for (uri, body) in responses {
            bmc.expect(Expect::get(uri, body));
        }
        let root = ServiceRoot::new(bmc.clone())
            .await
            .expect("service root wrapper");
        let mut errors = Vec::new();
        let result = collect_fabrics(&root, &mut errors).await;
        let fabrics = collect_domain(&mut errors, inventory_domain::FABRICS, result);
        (fabrics, errors)
    }

    const FABRICS_COLLECTION: (&str, &str) = (
        "/redfish/v1/Fabrics",
        r##"{"@odata.id":"/redfish/v1/Fabrics","@odata.type":"#FabricCollection.FabricCollection","Name":"Fabrics","Members":[{"@odata.id":"/redfish/v1/Fabrics/NVLink"}]}"##,
    );
    const NVLINK_FABRIC: (&str, &str) = (
        "/redfish/v1/Fabrics/NVLink",
        r##"{"@odata.id":"/redfish/v1/Fabrics/NVLink","@odata.type":"#Fabric.v1_3_0.Fabric","Id":"NVLink","Name":"NVLink Fabric","FabricType":"NVLink","UUID":"33333333-4444-5555-6666-777777777777","Status":{"State":"Enabled","Health":"OK"},"Switches":{"@odata.id":"/redfish/v1/Fabrics/NVLink/Switches"},"Links":{"ManagedBy":[{"@odata.id":"/redfish/v1/Managers/BMC_0"}]}}"##,
    );
    const SWITCHES_COLLECTION: (&str, &str) = (
        "/redfish/v1/Fabrics/NVLink/Switches",
        r##"{"@odata.id":"/redfish/v1/Fabrics/NVLink/Switches","@odata.type":"#SwitchCollection.SwitchCollection","Name":"Switches","Members":[{"@odata.id":"/redfish/v1/Fabrics/NVLink/Switches/NVSwitch_0"}]}"##,
    );

    #[tokio::test]
    async fn collects_fabric_and_nvswitch_inventory() {
        let (fabrics, errors) = collect_mock_fabrics(&[
            FABRICS_COLLECTION,
            NVLINK_FABRIC,
            SWITCHES_COLLECTION,
            (
                "/redfish/v1/Fabrics/NVLink/Switches/NVSwitch_0",
                r##"{"@odata.id":"/redfish/v1/Fabrics/NVLink/Switches/NVSwitch_0","@odata.type":"#Switch.v1_9_0.Switch","Id":"NVSwitch_0","Name":"NVSwitch 0","UUID":"44444444-5555-6666-7777-888888888888","SwitchType":"NVLink","Manufacturer":"NVIDIA","Model":"NVSwitch","SKU":"NVL-SKU","SerialNumber":"SW-SERIAL","PartNumber":"SW-PART","FirmwareVersion":"1.2.3","Location":{"PartLocation":{"ServiceLabel":"NVSwitch_0","LocationType":"Slot","LocationOrdinalValue":0}},"Oem":{"Nvidia":{"@odata.type":"#NvidiaSwitch.v1_4_0.NvidiaSwitch","DeviceId":"0x22A3","VendorId":"0x10DE","PCIeReferenceClockEnabled":true,"PPCIeModeEnabled":false,"SwitchIsolationMode":"SwitchCommunicationEnabled"}},"Status":{"State":"Enabled","Health":"OK"},"Links":{"Chassis":{"@odata.id":"/redfish/v1/Chassis/SwitchTray"}}}"##,
            ),
        ])
        .await;

        assert!(errors.is_empty(), "{errors:?}");
        assert_eq!(fabrics.len(), 1);
        let fabric = &fabrics[0];
        assert_eq!(fabric.id, "NVLink");
        assert_eq!(fabric.fabric_type.as_deref(), Some("NVLink"));
        assert_eq!(
            fabric.uuid.as_deref(),
            Some("33333333-4444-5555-6666-777777777777")
        );
        assert_eq!(
            fabric.status.as_ref().and_then(|s| s.state.as_deref()),
            Some("Enabled")
        );
        assert_eq!(
            fabric.links["ManagedBy"],
            vec![link_reference("/redfish/v1/Managers/BMC_0".to_string())]
        );

        assert_eq!(fabric.switches.len(), 1);
        let switch = &fabric.switches[0];
        assert_eq!(switch.id, "NVSwitch_0");
        assert_eq!(switch.switch_type.as_deref(), Some("NVLink"));
        assert_eq!(switch.sku.as_deref(), Some("NVL-SKU"));
        assert_eq!(switch.serial_number.as_deref(), Some("SW-SERIAL"));
        assert_eq!(switch.part_number.as_deref(), Some("SW-PART"));
        assert_eq!(switch.firmware_version.as_deref(), Some("1.2.3"));
        assert_eq!(
            switch
                .location
                .as_ref()
                .and_then(|location| location.part_location.as_ref())
                .and_then(|part| part.service_label.as_deref()),
            Some("NVSwitch_0")
        );
        assert_eq!(
            switch.links["Chassis"],
            vec![link_reference("/redfish/v1/Chassis/SwitchTray".to_string())]
        );
        let nvidia = switch.nvidia_switch.as_ref().expect("NVIDIA switch OEM");
        assert_eq!(nvidia.device_id.as_deref(), Some("0x22A3"));
        assert_eq!(nvidia.vendor_id.as_deref(), Some("0x10DE"));
        assert_eq!(nvidia.pcie_reference_clock_enabled, Some(true));
        assert_eq!(nvidia.protected_pcie_mode_enabled, Some(false));
        assert_eq!(
            nvidia.switch_isolation_mode.as_deref(),
            Some("SwitchCommunicationEnabled")
        );
    }

    #[tokio::test]
    async fn switch_failure_is_isolated_to_its_switch() {
        let (fabrics, errors) = collect_mock_fabrics(&[
            FABRICS_COLLECTION,
            NVLINK_FABRIC,
            (
                "/redfish/v1/Fabrics/NVLink/Switches",
                r##"{"@odata.id":"/redfish/v1/Fabrics/NVLink/Switches","@odata.type":"#SwitchCollection.SwitchCollection","Name":"Switches","Members":[{"@odata.id":"/redfish/v1/Fabrics/NVLink/Switches/bad"},{"@odata.id":"/redfish/v1/Fabrics/NVLink/Switches/NVSwitch_1"}]}"##,
            ),
            (
                "/redfish/v1/Fabrics/NVLink/Switches/bad",
                r##"{"@odata.id":"/redfish/v1/Fabrics/NVLink/Switches/bad","@odata.type":"#Switch.v1_9_0.Switch","Name":"missing id"}"##,
            ),
            (
                "/redfish/v1/Fabrics/NVLink/Switches/NVSwitch_1",
                r##"{"@odata.id":"/redfish/v1/Fabrics/NVLink/Switches/NVSwitch_1","@odata.type":"#Switch.v1_9_0.Switch","Id":"NVSwitch_1","Name":"NVSwitch 1"}"##,
            ),
        ])
        .await;

        assert_eq!(fabrics.len(), 1);
        assert_eq!(fabrics[0].switches.len(), 1);
        assert_eq!(fabrics[0].switches[0].id, "NVSwitch_1");
        assert_eq!(errors.len(), 1);
        assert_eq!(errors[0].domain, inventory_domain::FABRICS_SWITCHES);
        assert_eq!(errors[0].resource.as_deref(), Some("bad"));
    }

    #[tokio::test]
    async fn malformed_fabric_skips_only_that_fabric() {
        let (fabrics, errors) = collect_mock_fabrics(&[
            (
                "/redfish/v1/Fabrics",
                r##"{"@odata.id":"/redfish/v1/Fabrics","@odata.type":"#FabricCollection.FabricCollection","Name":"Fabrics","Members":[{"@odata.id":"/redfish/v1/Fabrics/Bad"},{"@odata.id":"/redfish/v1/Fabrics/NVLink"}]}"##,
            ),
            (
                "/redfish/v1/Fabrics/Bad",
                r##"{"@odata.id":"/redfish/v1/Fabrics/Bad","@odata.type":"#Fabric.v1_3_0.Fabric","Id":"Bad","Name":"Bad Fabric","FabricType":42}"##,
            ),
            (
                "/redfish/v1/Fabrics/NVLink",
                r##"{"@odata.id":"/redfish/v1/Fabrics/NVLink","@odata.type":"#Fabric.v1_3_0.Fabric","Id":"NVLink","Name":"NVLink Fabric"}"##,
            ),
        ])
        .await;

        assert_eq!(fabrics.len(), 1);
        assert_eq!(fabrics[0].id, "NVLink");
        assert_eq!(errors.len(), 1);
        assert_eq!(errors[0].domain, inventory_domain::FABRICS);
        assert_eq!(errors[0].resource.as_deref(), Some("Bad"));
    }

    #[tokio::test]
    async fn malformed_switch_type_is_a_switch_error() {
        let (fabrics, errors) = collect_mock_fabrics(&[
            FABRICS_COLLECTION,
            NVLINK_FABRIC,
            SWITCHES_COLLECTION,
            (
                "/redfish/v1/Fabrics/NVLink/Switches/NVSwitch_0",
                r##"{"@odata.id":"/redfish/v1/Fabrics/NVLink/Switches/NVSwitch_0","@odata.type":"#Switch.v1_9_0.Switch","Id":"NVSwitch_0","Name":"NVSwitch 0","SwitchType":{"unexpected":true}}"##,
            ),
        ])
        .await;

        assert_eq!(fabrics.len(), 1);
        assert!(fabrics[0].switches.is_empty());
        assert_eq!(errors.len(), 1);
        assert_eq!(errors[0].domain, inventory_domain::FABRICS_SWITCHES);
        assert_eq!(errors[0].resource.as_deref(), Some("NVSwitch_0"));
    }

    #[tokio::test]
    async fn fabric_collection_without_members_is_a_domain_error() {
        let (fabrics, errors) = collect_mock_fabrics(&[(
            "/redfish/v1/Fabrics",
            r##"{"@odata.id":"/redfish/v1/Fabrics","@odata.type":"#FabricCollection.FabricCollection","Name":"Fabrics"}"##,
        )])
        .await;

        assert!(fabrics.is_empty());
        assert_eq!(errors.len(), 1, "{errors:?}");
        assert_eq!(errors[0].domain, inventory_domain::FABRICS);
        assert!(errors[0].resource.is_none());
        assert!(errors[0].message.contains("Members"), "{errors:?}");
    }

    #[tokio::test]
    async fn malformed_nvidia_switch_oem_keeps_standard_inventory() {
        let (fabrics, errors) = collect_mock_fabrics(&[
            FABRICS_COLLECTION,
            NVLINK_FABRIC,
            SWITCHES_COLLECTION,
            (
                "/redfish/v1/Fabrics/NVLink/Switches/NVSwitch_0",
                r##"{"@odata.id":"/redfish/v1/Fabrics/NVLink/Switches/NVSwitch_0","@odata.type":"#Switch.v1_9_0.Switch","Id":"NVSwitch_0","Name":"NVSwitch 0","SerialNumber":"SW-SERIAL","Oem":{"Nvidia":{"@odata.type":"#NvidiaSwitch.v1_4_0.NvidiaSwitch","PPCIeModeEnabled":"invalid"}}}"##,
            ),
        ])
        .await;

        assert_eq!(fabrics[0].switches.len(), 1);
        let switch = &fabrics[0].switches[0];
        assert_eq!(switch.serial_number.as_deref(), Some("SW-SERIAL"));
        assert!(switch.nvidia_switch.is_none());
        assert_eq!(errors.len(), 1);
        assert_eq!(errors[0].domain, inventory_domain::FABRICS_SWITCHES);
        assert_eq!(errors[0].resource.as_deref(), Some("NVSwitch_0"));
        assert!(errors[0].message.starts_with("NVIDIA switch OEM data:"));
    }

    const FABRIC_SERVICE_ROOT: &str = r##"{
        "@odata.id": "/redfish/v1",
        "@odata.type": "#ServiceRoot.v1_15_0.ServiceRoot",
        "Id": "RootService",
        "Name": "Root Service",
        "RedfishVersion": "1.15.1",
        "Links": {
            "Sessions": {"@odata.id": "/redfish/v1/SessionService/Sessions"}
        },
        "Vendor": "AMI",
        "Product": "Mock BMC",
        "Fabrics": {"@odata.id": "/redfish/v1/Fabrics"}
    }"##;

    const SERVICE_ROOT: &str = r##"{
        "@odata.id": "/redfish/v1",
        "@odata.type": "#ServiceRoot.v1_15_0.ServiceRoot",
        "Id": "RootService",
        "Name": "Root Service",
        "RedfishVersion": "1.15.1",
        "UUID": "870155dd-d474-aca8-11ee-6e809af87c0c",
        "Links": {
            "Sessions": {"@odata.id": "/redfish/v1/SessionService/Sessions"}
        },
        "Vendor": "AMI",
        "Product": "Mock BMC",
        "Managers": {"@odata.id": "/redfish/v1/Managers"},
        "Systems": {"@odata.id": "/redfish/v1/Systems"},
        "Chassis": {"@odata.id": "/redfish/v1/Chassis"},
        "UpdateService": {"@odata.id": "/redfish/v1/UpdateService"}
    }"##;

    const MANAGERS_COLLECTION: &str = r##"{
        "@odata.id": "/redfish/v1/Managers",
        "@odata.type": "#ManagerCollection.ManagerCollection",
        "Id": "Managers",
        "Name": "Managers",
        "Members@odata.count": 1,
        "Members": [{"@odata.id": "/redfish/v1/Managers/BMC_0"}]
    }"##;

    const MANAGER: &str = r##"{
        "@odata.id": "/redfish/v1/Managers/BMC_0",
        "@odata.type": "#Manager.v1_15_0.Manager",
        "Id": "BMC_0",
        "Name": "BMC",
        "Description": "  Baseboard management controller  ",
        "ManagerType": "BMC",
        "UUID": "11111111-2222-3333-4444-555555555555",
        "Model": "OpenBMC",
        "FirmwareVersion": "3.19.00",
        "Links": {
            "ManagerForServers": [
                {"@odata.id": "https://bmc.example.com/redfish/v1/Systems/System?view=detail#item"}
            ]
        }
    }"##;

    const MALFORMED_MANAGER: &str = r##"{
        "@odata.id": "/redfish/v1/Managers/BMC_0",
        "@odata.type": "#Manager.v1_15_0.Manager",
        "Id": "BMC_0",
        "Name": "BMC",
        "ManagerType": 1
    }"##;

    const SYSTEMS_COLLECTION: &str = r##"{
        "@odata.id": "/redfish/v1/Systems",
        "@odata.type": "#ComputerSystemCollection.ComputerSystemCollection",
        "Id": "Systems",
        "Name": "Systems",
        "Members@odata.count": 1,
        "Members": [{"@odata.id": "/redfish/v1/Systems/System"}]
    }"##;

    const SYSTEM: &str = r##"{
        "@odata.id": "/redfish/v1/Systems/System",
        "@odata.type": "#ComputerSystem.v1_15_0.ComputerSystem",
        "Id": "DGX",
        "Name": "System",
        "UUID": "9AEE71E4-6E80-11EE-A8AC-74D4DD550187",
        "Manufacturer": "NVIDIA",
        "Model": "GB200",
        "SKU": "SKU-GB200",
        "SerialNumber": "SN123",
        "BiosVersion": "3A21",
        "HostName": "node-a.example.com",
        "AssetTag": "rack-12-tray-3",
        "PowerState": "On",
        "ProcessorSummary": {
            "Count": 1,
            "CoreCount": 72,
            "Model": "Grace"
        },
        "MemorySummary": {
            "TotalSystemMemoryGiB": 480
        },
        "Status": {
            "State": "Enabled",
            "Health": "OK",
            "HealthRollup": "Warning"
        }
    }"##;

    const SYSTEM_WITH_PROCESSORS: &str = r##"{
        "@odata.id": "/redfish/v1/Systems/System",
        "@odata.type": "#ComputerSystem.v1_15_0.ComputerSystem",
        "Id": "DGX",
        "Name": "System",
        "UUID": "9AEE71E4-6E80-11EE-A8AC-74D4DD550187",
        "Manufacturer": "NVIDIA",
        "Model": "GB200",
        "SKU": "SKU-GB200",
        "SerialNumber": "SN123",
        "BiosVersion": "3A21",
        "HostName": "node-a.example.com",
        "AssetTag": "rack-12-tray-3",
        "PowerState": "On",
        "ProcessorSummary": {
            "Count": 1,
            "CoreCount": 72,
            "Model": "Grace"
        },
        "Processors": {"@odata.id": "/redfish/v1/Systems/System/Processors"},
        "Links": {
            "Chassis": [{"@odata.id": "/redfish/v1/Chassis/HGX_GPU_SXM_0"}],
            "ManagedBy": [{"@odata.id": "/redfish/v1/Managers/BMC_0"}]
        },
        "MemorySummary": {
            "TotalSystemMemoryGiB": 480
        },
        "Status": {
            "State": "Enabled",
            "Health": "OK",
            "HealthRollup": "Warning"
        }
    }"##;

    const PROCESSORS_COLLECTION: &str = r##"{
        "@odata.id": "/redfish/v1/Systems/System/Processors",
        "@odata.type": "#ProcessorCollection.ProcessorCollection",
        "Id": "Processors",
        "Name": "Processors",
        "Members@odata.count": 1,
        "Members": [{"@odata.id": "/redfish/v1/Systems/System/Processors/CPU1"}]
    }"##;

    const PROCESSOR_NON_GPU_OEM: &str = r##"{
        "@odata.id": "/redfish/v1/Systems/System/Processors/CPU1",
        "@odata.type": "#Processor.v1_20_0.Processor",
        "Id": "CPU1",
        "Name": "CPU 1",
        "Oem": {
            "Nvidia": {"@odata.type": "#NvidiaProcessor.v1_7_0.NvidiaCPU"},
            "OtherVendor": {"Ignored": true}
        }
    }"##;

    const PROCESSOR_MALFORMED_LPU_OEM: &str = r##"{
        "@odata.id": "/redfish/v1/Systems/System/Processors/CPU1",
        "@odata.type": "#Processor.v1_20_0.Processor",
        "Id": "CPU1",
        "Name": "CPU 1",
        "Oem": {
            "Nvidia": {
                "@odata.type": "#NvidiaProcessor.v1_7_0.NvidiaLPU",
                "BootStatusCode": "something"
            }
        }
    }"##;

    const PROCESSOR_MALFORMED_GPU_OEM: &str = r##"{
        "@odata.id": "/redfish/v1/Systems/System/Processors/CPU1",
        "@odata.type": "#Processor.v1_20_0.Processor",
        "Id": "CPU1",
        "Name": "CPU 1",
        "Oem": {
            "Nvidia": {
                "@odata.type": "#NvidiaProcessor.v1_7_0.NvidiaGPU",
                "MIGModeEnabled": "not-a-boolean"
            }
        }
    }"##;

    const PROCESSOR: &str = r##"{
        "@odata.id": "/redfish/v1/Systems/System/Processors/CPU1",
        "@odata.type": "#Processor.v1_20_0.Processor",
        "Id": "CPU1",
        "Name": "CPU 1",
        "Socket": "CPU Socket 1",
        "ProcessorType": "CPU",
        "ProcessorArchitecture": "ARM",
        "InstructionSet": "ARM-A64",
        "Manufacturer": "NVIDIA",
        "Model": "Grace",
        "UUID": "22222222-3333-4444-5555-666666666666",
        "SerialNumber": "CPU-SN123",
        "PartNumber": "CPU-PN123",
        "Location": {
            "PartLocationContext": "HGX_Chassis_0/ProcessorModule_0",
            "PartLocation": {
                "ServiceLabel": "GPU_SXM_1",
                "LocationType": "Slot",
                "LocationOrdinalValue": 1,
                "Orientation": "LeftToRight"
            }
        },
        "Oem": {
            "Nvidia": {
                "@odata.type": "#NvidiaProcessor.v1_6_0.NvidiaGPU",
                "MIGModeEnabled": false,
                "DefaultBoostClockSpeedMHz": 1200,
                "MaxBoostClockSpeedMHz": 1800,
                "CCModeEnabled": true,
                "TotalNumberNVLinks": 18,
                "MNNVLinkTopology": {
                    "ChassisSerialNumber": "chassis-serial",
                    "HostID": 17,
                    "IBGUID": "ib-guid",
                    "PeerType": "Direct",
                    "TraySlotIndex": 2,
                    "SystemGUID": "system-guid"
                }
            },
            "OtherVendor": {"Ignored": true}
        },
        "MaxSpeedMHz": 3600,
        "TotalCores": 72,
        "TotalThreads": 72,
        "Links": {
            "Chassis": {"@odata.id": "/redfish/v1/Chassis/HGX_GPU_SXM_0"}
        },
        "Status": {
            "State": "Enabled",
            "Health": "OK",
            "HealthRollup": "OK"
        }
    }"##;

    const SYSTEM_WITH_SECURE_BOOT: &str = r##"{
        "@odata.id": "/redfish/v1/Systems/System",
        "@odata.type": "#ComputerSystem.v1_15_0.ComputerSystem",
        "Id": "DGX",
        "Name": "System",
        "UUID": "9AEE71E4-6E80-11EE-A8AC-74D4DD550187",
        "SecureBoot": {"@odata.id": "/redfish/v1/Systems/System/SecureBoot"}
    }"##;

    const MALFORMED_SECURE_BOOT: &str = r##"{
        "@odata.id": "/redfish/v1/Systems/System/SecureBoot",
        "@odata.type": "#SecureBoot.v1_1_0.SecureBoot",
        "Id": "SecureBoot",
        "Name": "Secure Boot",
        "SecureBootEnable": "enabled"
    }"##;

    const CHASSIS_COLLECTION: &str = r##"{
        "@odata.id": "/redfish/v1/Chassis",
        "@odata.type": "#ChassisCollection.ChassisCollection",
        "Id": "Chassis",
        "Name": "Chassis",
        "Members@odata.count": 1,
        "Members": [{"@odata.id": "/redfish/v1/Chassis/HGX_GPU_SXM_0"}]
    }"##;

    const CHASSIS: &str = r##"{
        "@odata.id": "/redfish/v1/Chassis/HGX_GPU_SXM_0",
        "@odata.type": "#Chassis.v1_25_0.Chassis",
        "Id": "HGX_GPU_SXM_0",
        "Name": "BMC Chassis",
        "ChassisType": "Module",
        "Manufacturer": "NVIDIA",
        "Model": "BMC",
        "SerialNumber": "CH123",
        "PartNumber": "PN123",
        "Status": {
            "State": "Enabled",
            "Health": "Critical",
            "HealthRollup": "Critical",
            "Conditions": [{
                "Message": "The resource property CPU_1 Firmware has detected errors of type 'EC_STRAP_MISMATCH'.",
                "MessageArgs": ["CPU_1 Firmware", "EC_STRAP_MISMATCH"],
                "MessageId": "ResourceEvent.1.0.ResourceErrorsDetected",
                "Oem": {
                    "Nvidia": {
                        "@odata.type": "#NvidiaLogEntry.v1_1_0.NvidiaLogEntry",
                        "Device": "CPU_1",
                        "ErrorId": "CPU_EC_STRAP_MISMATCH"
                    }
                },
                "OriginOfCondition": {"@odata.id": "/redfish/v1/Chassis/HGX_CPU_1"},
                "Resolution": "Reset the component; collect logs and reflash if the problem remains.",
                "Severity": "Critical",
                "Timestamp": "2026-08-25T20:51:47Z"
            }]
        },
        "PCIeDevices": {"@odata.id": "/redfish/v1/Chassis/HGX_GPU_SXM_0/PCIeDevices"},
        "Links": {
            "ComputerSystems": [{"@odata.id": "/redfish/v1/Systems/System"}],
            "ManagedBy": [{"@odata.id": "/redfish/v1/Managers/BMC_0"}]
        },
        "Location": {
            "PartLocation": {"ServiceLabel": "Bay 1"},
            "Placement": {"Rack": "R12", "Row": "A", "RackOffset": 7}
        }
    }"##;

    const PCIE_DEVICES_COLLECTION: &str = r##"{
        "@odata.id": "/redfish/v1/Chassis/HGX_GPU_SXM_0/PCIeDevices",
        "@odata.type": "#PCIeDeviceCollection.PCIeDeviceCollection",
        "Id": "PCIeDevices",
        "Name": "PCIe Devices",
        "Members@odata.count": 1,
        "Members": [{"@odata.id": "/redfish/v1/Chassis/HGX_GPU_SXM_0/PCIeDevices/GPU1"}]
    }"##;

    const MALFORMED_PCIE_DEVICES_COLLECTION: &str = r##"{
        "@odata.id": "/redfish/v1/Chassis/HGX_GPU_SXM_0/PCIeDevices",
        "@odata.type": "#PCIeDeviceCollection.PCIeDeviceCollection",
        "Id": "PCIeDevices",
        "Name": "PCIe Devices",
        "Members@odata.count": 1,
        "Members": 1
    }"##;

    const PCIE_DEVICE: &str = r##"{
        "@odata.id": "/redfish/v1/Chassis/HGX_GPU_SXM_0/PCIeDevices/GPU1",
        "@odata.type": "#PCIeDevice.v1_16_0.PCIeDevice",
        "Id": "GPU1",
        "Name": "GPU 1",
        "Manufacturer": "NVIDIA",
        "Model": "B200",
        "SKU": "B200-SXM",
        "SerialNumber": "GPU-SN456",
        "PartNumber": "GPU-PN456",
        "DeviceType": "SingleFunction",
        "FirmwareVersion": "96.00.5E.00.02",
        "Links": {
            "Chassis": [{"@odata.id": "/redfish/v1/Chassis/HGX_GPU_SXM_0"}]
        },
        "Status": {
            "State": "Enabled",
            "Health": "OK",
            "Conditions": []
        }
    }"##;

    const UPDATE_SERVICE: &str = r##"{
        "@odata.id": "/redfish/v1/UpdateService",
        "@odata.type": "#UpdateService.v1_14_0.UpdateService",
        "Id": "UpdateService",
        "Name": "Update Service",
        "FirmwareInventory": {"@odata.id": "/redfish/v1/UpdateService/FirmwareInventory"}
    }"##;

    const FIRMWARE_COLLECTION: &str = r##"{
        "@odata.id": "/redfish/v1/UpdateService/FirmwareInventory",
        "@odata.type": "#SoftwareInventoryCollection.SoftwareInventoryCollection",
        "Id": "FirmwareInventory",
        "Name": "Firmware Inventory",
        "Members@odata.count": 1,
        "Members": [{"@odata.id": "/redfish/v1/UpdateService/FirmwareInventory/BMC"}]
    }"##;

    const FIRMWARE: &str = r##"{
        "@odata.id": "/redfish/v1/UpdateService/FirmwareInventory/BMC",
        "@odata.type": "#SoftwareInventory.v1_8_0.SoftwareInventory",
        "Id": "BMC",
        "Name": "BMC",
        "RelatedItem": [{"@odata.id": "/redfish/v1/Managers/BMC_0#/Firmware"}],
        "Version": "3.19.00",
        "ReleaseDate": "2026-01-01T00:00:00Z",
        "Links": {
            "ActiveTargets": [{"@odata.id": "/redfish/v1/Managers/BMC_0"}]
        },
        "Status": {
            "State": "StandbyOffline",
            "Health": "Critical",
            "HealthRollup": "Warning"
        }
    }"##;

    const MALFORMED_FIRMWARE: &str = r##"{
        "@odata.id": "/redfish/v1/UpdateService/FirmwareInventory/BMC",
        "@odata.type": "#SoftwareInventory.v1_8_0.SoftwareInventory",
        "Id": 1,
        "Name": "BMC"
    }"##;
}
