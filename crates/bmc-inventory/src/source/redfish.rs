use std::collections::BTreeMap;
use std::error::Error;
use std::sync::Arc;
use std::time::Duration;

use async_trait::async_trait;
use bmc_dispatch::{Dispatcher, DispatchingHttpClient};
use nv_redfish::bmc_http::{BmcCredentials, CacheSettings, HttpBmc};
use nv_redfish::core::{EntityTypeRef, NavProperty, ReferenceLeaf};
use nv_redfish::{Bmc, Resource, ServiceRoot};
use time::OffsetDateTime;
use time::format_description::well_known::Rfc3339;
use url::Url;

use super::{
    ChassisData, ChassisLocationData, ConditionData, FirmwareData, InventoryErrorData,
    InventorySource, LinkData, ManagerData, PcieDeviceData, ProcessorData, RedfishReport,
    ServiceRootData, SourceRecord, StatusData, SystemData, inventory_domain,
};
use crate::config::Target;

type RedfishBmc = HttpBmc<DispatchingHttpClient<nv_redfish::bmc_http::reqwest::Client>>;

const MAX_REDIRECTS: usize = 3;

#[derive(Clone, Debug, PartialEq, Eq)]
enum PcieTraversalPolicy {
    All,
    Viking,
    SystemId(String),
    None,
}

pub struct RedfishSource {
    client: nv_redfish::bmc_http::reqwest::Client,
    dispatcher: Arc<Dispatcher>,
}

impl RedfishSource {
    pub fn new(
        insecure_tls: bool,
        request_timeout: Duration,
        dispatcher: Arc<Dispatcher>,
    ) -> Result<Self, Box<dyn Error>> {
        if request_timeout.is_zero() {
            return Err("request timeout must be greater than zero".into());
        }
        let connect_timeout = request_timeout.min(Duration::from_secs(10));
        let client = nv_redfish::bmc_http::reqwest::Client::with_params(
            nv_redfish::bmc_http::reqwest::ClientParams::new()
                .timeout(request_timeout)
                .connect_timeout(connect_timeout)
                .accept_invalid_certs(insecure_tls)
                // Some BMCs canonicalize the service root (for example,
                // /redfish/v1 -> /redfish/v1/). Keep redirects bounded while
                // allowing those standards-compatible endpoints to work.
                .max_redirects(MAX_REDIRECTS),
        )?;
        Ok(Self { client, dispatcher })
    }

    async fn collect_report(&self, target: &Target) -> Result<RedfishReport, String> {
        let root = self
            .service_root(target)
            .await
            .map_err(|err| err.to_string())?;
        collect_report_from_root(&root).await
    }

    async fn service_root(
        &self,
        target: &Target,
    ) -> Result<ServiceRoot<RedfishBmc>, Box<dyn Error>> {
        let url = bmc_url(&target.address)?;
        let gate = self
            .dispatcher
            .inventory_gate(&target.mac)
            .ok_or_else(|| format!("no dispatcher endpoint for target {:?}", target.mac))?;
        let bmc = Arc::new(HttpBmc::new(
            DispatchingHttpClient::new(self.client.clone(), gate),
            url,
            BmcCredentials::new(target.username.clone(), target.password.clone()),
            CacheSettings::with_capacity(32),
        ));
        Ok(ServiceRoot::new(bmc).await?)
    }
}

#[async_trait]
impl InventorySource for RedfishSource {
    async fn collect(&self, target: Target) -> SourceRecord {
        let collected_at = now_rfc3339();
        let result = self.collect_report(&target).await;
        SourceRecord {
            target,
            collected_at,
            result,
        }
    }
}

async fn collect_report_from_root<B: Bmc>(root: &ServiceRoot<B>) -> Result<RedfishReport, String> {
    let raw_root = root.root.as_ref();
    let service_root = ServiceRootData {
        redfish_version: raw_root.redfish_version.clone(),
        uuid: raw_root.uuid.flatten().map(|uuid| uuid.to_string()),
        vendor: raw_root.vendor.clone().flatten(),
        product: raw_root.product.clone().flatten(),
    };

    let mut errors = Vec::new();
    let managers = match collect_managers(root).await {
        Ok(managers) => managers,
        Err(err) => {
            push_inventory_error(&mut errors, inventory_domain::MANAGERS, None, err);
            Vec::new()
        }
    };
    let systems = match collect_systems(root, &mut errors).await {
        Ok(systems) => systems,
        Err(err) => {
            push_inventory_error(&mut errors, inventory_domain::SYSTEMS, None, err);
            Vec::new()
        }
    };
    let chassis = match collect_chassis(root, &service_root, &systems, &mut errors).await {
        Ok(chassis) => chassis,
        Err(err) => {
            push_inventory_error(&mut errors, inventory_domain::CHASSIS, None, err);
            Vec::new()
        }
    };
    let firmware = match collect_firmware(root).await {
        Ok(firmware) => firmware,
        Err(err) => {
            push_inventory_error(&mut errors, inventory_domain::FIRMWARE, None, err);
            Vec::new()
        }
    };

    Ok(RedfishReport {
        service_root,
        managers,
        systems,
        chassis,
        firmware,
        errors,
    })
}

async fn collect_managers<B: Bmc>(root: &ServiceRoot<B>) -> Result<Vec<ManagerData>, String> {
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
        out.push(ManagerData {
            id: manager.id().to_string(),
            odata_id: Some(manager.odata_id().to_string()),
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
    errors: &mut Vec<InventoryErrorData>,
) -> Result<Vec<SystemData>, String> {
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
        let system_id = system.id().to_string();
        let raw = system.raw();
        out.push(SystemData {
            id: system_id.clone(),
            odata_id: Some(system.odata_id().to_string()),
            uuid: raw.uuid.flatten().map(|uuid| uuid.to_string()),
            manufacturer: raw.manufacturer.clone().flatten(),
            model: raw.model.clone().flatten(),
            sku: raw.sku.clone().flatten(),
            serial_number: raw.serial_number.clone().flatten(),
            bios_version: raw.bios_version.clone().flatten(),
            host_name: raw.host_name.clone().flatten(),
            asset_tag: raw.asset_tag.clone().flatten(),
            power_state: raw.power_state.flatten().and_then(redfish_enum_string),
            status: status_data(raw.status.as_ref()),
            cpu_count: raw
                .processor_summary
                .as_ref()
                .and_then(|summary| summary.count)
                .flatten(),
            cpu_core_count: raw
                .processor_summary
                .as_ref()
                .and_then(|summary| summary.core_count)
                .flatten(),
            cpu_model: raw
                .processor_summary
                .as_ref()
                .and_then(|summary| summary.model.clone())
                .flatten(),
            memory_gib: raw
                .memory_summary
                .as_ref()
                .and_then(|summary| summary.total_system_memory_gi_b)
                .flatten(),
            secure_boot_enabled: collect_secure_boot_enabled(&system, &system_id, errors).await,
            processors: collect_processors(&system, &system_id, errors).await,
            links: system_links(raw.links.as_ref()),
        });
    }
    Ok(out)
}

async fn collect_chassis<B: Bmc>(
    root: &ServiceRoot<B>,
    service_root: &ServiceRootData,
    systems: &[SystemData],
    errors: &mut Vec<InventoryErrorData>,
) -> Result<Vec<ChassisData>, String> {
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
    let pcie_traversal_policy = pcie_traversal_policy(service_root, systems);
    let mut out = Vec::with_capacity(chassis.len());
    for chassis in chassis {
        let chassis_id = chassis.id().to_string();
        let raw = chassis.raw();
        out.push(ChassisData {
            id: chassis_id.clone(),
            odata_id: Some(chassis.odata_id().to_string()),
            chassis_type: redfish_enum_string(raw.chassis_type),
            manufacturer: raw.manufacturer.clone().flatten(),
            model: raw.model.clone().flatten(),
            sku: raw.sku.clone().flatten(),
            serial_number: raw.serial_number.clone().flatten(),
            part_number: raw.part_number.clone().flatten(),
            asset_tag: raw.asset_tag.clone().flatten(),
            power_state: raw.power_state.flatten().and_then(redfish_enum_string),
            status: status_data(raw.status.as_ref()),
            location: chassis_location(raw.location.as_ref()),
            pcie_devices: if should_collect_pcie_devices_for_chassis(
                &pcie_traversal_policy,
                &chassis_id,
            ) {
                collect_pcie_devices(&chassis, &chassis_id, errors).await
            } else {
                Vec::new()
            },
            links: chassis_links(raw.links.as_ref()),
        });
    }
    Ok(out)
}

fn pcie_traversal_policy(
    service_root: &ServiceRootData,
    systems: &[SystemData],
) -> PcieTraversalPolicy {
    let vendor = service_root.vendor.as_deref();
    let selected_system_id =
        selected_system_for_pcie_traversal(systems).map(|system| system.id.clone());

    if vendor_matches(vendor, "AMI") && selected_system_id.as_deref() == Some("DGX") {
        return PcieTraversalPolicy::Viking;
    }
    if vendor_matches(vendor, "AMI")
        || vendor_matches(vendor, "Dell")
        || vendor_matches(vendor, "HPE")
        || vendor_matches(vendor, "Supermicro")
        || vendor_matches(vendor, "Lenovo")
    {
        return selected_system_id
            .map(PcieTraversalPolicy::SystemId)
            .unwrap_or(PcieTraversalPolicy::None);
    }

    PcieTraversalPolicy::All
}

fn selected_system_for_pcie_traversal(systems: &[SystemData]) -> Option<&SystemData> {
    systems
        .iter()
        .find(|system| system.bios_version.is_some())
        .or_else(|| systems.first())
}

fn should_collect_pcie_devices_for_chassis(policy: &PcieTraversalPolicy, chassis_id: &str) -> bool {
    match policy {
        PcieTraversalPolicy::All => true,
        PcieTraversalPolicy::Viking => {
            chassis_id.starts_with("HGX_GPU_SXM") || chassis_id.starts_with("HGX_NVSwitch")
        }
        PcieTraversalPolicy::SystemId(system_id) => chassis_id == system_id,
        PcieTraversalPolicy::None => false,
    }
}

fn vendor_matches(vendor: Option<&str>, expected: &str) -> bool {
    vendor.is_some_and(|vendor| vendor.eq_ignore_ascii_case(expected))
}

fn chassis_location(
    location: Option<&nv_redfish::schema::resource::Location>,
) -> Option<ChassisLocationData> {
    let location = location?;
    let part = location.part_location.as_ref();
    let placement = location.placement.as_ref();
    let data = ChassisLocationData {
        service_label: part.and_then(|p| p.service_label.clone().flatten()),
        rack: placement.and_then(|p| p.rack.clone().flatten()),
        row: placement.and_then(|p| p.row.clone().flatten()),
        room: placement.and_then(|p| p.room.clone().flatten()),
        rack_offset: placement.and_then(|p| p.rack_offset.flatten()),
    };
    if data.service_label.is_none()
        && data.rack.is_none()
        && data.row.is_none()
        && data.room.is_none()
        && data.rack_offset.is_none()
    {
        None
    } else {
        Some(data)
    }
}

async fn collect_secure_boot_enabled<B: Bmc>(
    system: &nv_redfish::computer_system::ComputerSystem<B>,
    system_id: &str,
    errors: &mut Vec<InventoryErrorData>,
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
    errors: &mut Vec<InventoryErrorData>,
) -> Vec<ProcessorData> {
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
        out.push(ProcessorData {
            id: processor.id().to_string(),
            odata_id: Some(processor.odata_id().to_string()),
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
            max_speed_mhz: raw.max_speed_mhz.flatten(),
            total_cores: raw.total_cores.flatten(),
            total_threads: raw.total_threads.flatten(),
            status: status_data(raw.status.as_ref()),
            links: processor_links(raw.links.as_ref()),
        });
    }
    out
}

async fn collect_pcie_devices<B: Bmc>(
    chassis: &nv_redfish::chassis::Chassis<B>,
    chassis_id: &str,
    errors: &mut Vec<InventoryErrorData>,
) -> Vec<PcieDeviceData> {
    let devices = match chassis.pcie_devices().await {
        Ok(Some(collection)) => match collection.members().await {
            Ok(devices) => devices,
            Err(err) => {
                push_inventory_error(
                    errors,
                    inventory_domain::CHASSIS_PCIE_DEVICES,
                    Some(chassis_id.to_string()),
                    err,
                );
                Vec::new()
            }
        },
        Ok(None) => Vec::new(),
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
        out.push(PcieDeviceData {
            id: device.id().to_string(),
            odata_id: Some(device.odata_id().to_string()),
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

async fn collect_firmware<B: Bmc>(root: &ServiceRoot<B>) -> Result<Vec<FirmwareData>, String> {
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
            FirmwareData {
                related_item: raw.related_item.as_deref().map(reference_targets),
                service_id: "FirmwareInventory".to_string(),
                id: inventory.id().to_string(),
                odata_id: Some(inventory.odata_id().to_string()),
                name: raw.base.name.clone(),
                version: raw.version.clone().flatten(),
                release_date: raw.release_date.flatten().map(|date| date.to_string()),
                status: status_data(raw.status.as_ref()),
                links: firmware_links(raw.links.as_ref()),
            }
        })
        .collect())
}

/// Use the dependency's Serde names, not its snake_case convenience names.
/// Unknown tokens have already been discarded by nv-redfish's enum parser.
fn redfish_enum_string(value: impl serde::Serialize) -> Option<String> {
    match serde_json::to_value(value).ok()? {
        serde_json::Value::String(value) if value != "UnsupportedValue" => Some(value),
        _ => None,
    }
}

fn manager_links(links: Option<&nv_redfish::schema::manager::Links>) -> BTreeMap<String, LinkData> {
    let mut out = BTreeMap::new();
    let Some(links) = links else {
        return out;
    };
    push_nav_links(
        &mut out,
        "ManagerForServers",
        links.manager_for_servers.as_deref(),
    );
    push_nav_links(
        &mut out,
        "ManagerForChassis",
        links.manager_for_chassis.as_deref(),
    );
    push_nav_link(
        &mut out,
        "ManagerInChassis",
        links.manager_in_chassis.as_ref(),
    );
    push_reference_links(
        &mut out,
        "ManagerForSwitches",
        links.manager_for_switches.as_deref(),
    );
    push_nav_link(
        &mut out,
        "ActiveSoftwareImage",
        links.active_software_image.as_ref(),
    );
    push_nav_links(&mut out, "SoftwareImages", links.software_images.as_deref());
    push_nav_links(&mut out, "ManagedBy", links.managed_by.as_deref());
    push_nav_links(
        &mut out,
        "ManagerForManagers",
        links.manager_for_managers.as_deref(),
    );
    push_nav_link(
        &mut out,
        "SelectedNetworkPort",
        links.selected_network_port.as_ref(),
    );
    push_reference_links(
        &mut out,
        "ManagerForFabrics",
        links.manager_for_fabrics.as_deref(),
    );
    out
}

fn system_links(
    links: Option<&nv_redfish::schema::computer_system::Links>,
) -> BTreeMap<String, LinkData> {
    let mut out = BTreeMap::new();
    let Some(links) = links else {
        return out;
    };
    push_nav_links(&mut out, "Chassis", links.chassis.as_deref());
    push_nav_links(&mut out, "ManagedBy", links.managed_by.as_deref());
    push_reference_links(&mut out, "PoweredBy", links.powered_by.as_deref());
    push_reference_links(&mut out, "CooledBy", links.cooled_by.as_deref());
    push_reference_links(&mut out, "Endpoints", links.endpoints.as_deref());
    push_reference_links(&mut out, "ResourceBlocks", links.resource_blocks.as_deref());
    push_nav_links(
        &mut out,
        "ConsumingComputerSystems",
        links.consuming_computer_systems.as_deref(),
    );
    push_nav_links(
        &mut out,
        "SupplyingComputerSystems",
        links.supplying_computer_systems.as_deref(),
    );
    push_nav_links(
        &mut out,
        "OffloadedNetworkDeviceFunctions",
        links.offloaded_network_device_functions.as_deref(),
    );
    push_reference_links(
        &mut out,
        "TrustedComponents",
        links.trusted_components.as_deref(),
    );
    push_nav_link(
        &mut out,
        "HostingComputerSystem",
        links.hosting_computer_system.as_ref(),
    );
    push_nav_links(
        &mut out,
        "VirtualMachines",
        links.virtual_machines.as_deref(),
    );
    out
}

fn chassis_links(links: Option<&nv_redfish::schema::chassis::Links>) -> BTreeMap<String, LinkData> {
    let mut out = BTreeMap::new();
    let Some(links) = links else {
        return out;
    };
    push_nav_links(
        &mut out,
        "ComputerSystems",
        links.computer_systems.as_deref(),
    );
    push_nav_links(&mut out, "ManagedBy", links.managed_by.as_deref());
    push_nav_link(&mut out, "ContainedBy", links.contained_by.as_ref());
    push_nav_links(&mut out, "Contains", links.contains.as_deref());
    push_reference_links(&mut out, "PoweredBy", links.powered_by.as_deref());
    push_reference_links(&mut out, "CooledBy", links.cooled_by.as_deref());
    push_nav_links(
        &mut out,
        "ManagersInChassis",
        links.managers_in_chassis.as_deref(),
    );
    push_nav_links(&mut out, "Drives", links.drives.as_deref());
    push_nav_links(&mut out, "Storage", links.storage.as_deref());
    push_nav_links(&mut out, "PCIeDevices", links.pcie_devices.as_deref());
    push_reference_links(&mut out, "ResourceBlocks", links.resource_blocks.as_deref());
    push_reference_links(&mut out, "Switches", links.switches.as_deref());
    push_nav_links(&mut out, "Processors", links.processors.as_deref());
    push_reference_link(&mut out, "Facility", links.facility.as_ref());
    push_reference_links(&mut out, "Cables", links.cables.as_deref());
    push_reference_links(&mut out, "PowerOutlets", links.power_outlets.as_deref());
    push_nav_links(&mut out, "PowerSupplies", links.power_supplies.as_deref());
    push_nav_links(&mut out, "Fans", links.fans.as_deref());
    push_nav_link(
        &mut out,
        "PowerDistribution",
        links.power_distribution.as_ref(),
    );
    push_reference_links(&mut out, "CoolingUnits", links.cooling_units.as_deref());
    push_reference_links(
        &mut out,
        "ConnectedCoolingLoops",
        links.connected_cooling_loops.as_deref(),
    );
    push_reference_links(
        &mut out,
        "AutomationNodes",
        links.automation_nodes.as_deref(),
    );
    out
}

fn processor_links(
    links: Option<&nv_redfish::schema::processor::Links>,
) -> BTreeMap<String, LinkData> {
    let mut out = BTreeMap::new();
    let Some(links) = links else {
        return out;
    };
    push_nav_link(&mut out, "Chassis", links.chassis.as_ref());
    push_reference_links(&mut out, "Endpoints", links.endpoints.as_deref());
    push_nav_links(
        &mut out,
        "ConnectedProcessors",
        links.connected_processors.as_deref(),
    );
    push_nav_link(&mut out, "PCIeDevice", links.pcie_device.as_ref());
    push_reference_links(&mut out, "PCIeFunctions", links.pcie_functions.as_deref());
    push_nav_links(&mut out, "Memory", links.memory.as_deref());
    push_reference_link(
        &mut out,
        "GraphicsController",
        links.graphics_controller.as_ref(),
    );
    push_nav_links(
        &mut out,
        "NetworkDeviceFunctions",
        links.network_device_functions.as_deref(),
    );
    push_reference_links(&mut out, "FabricAdapters", links.fabric_adapters.as_deref());
    out
}

fn pcie_device_links(
    links: Option<&nv_redfish::schema::pcie_device::Links>,
) -> BTreeMap<String, LinkData> {
    let mut out = BTreeMap::new();
    let Some(links) = links else {
        return out;
    };
    push_nav_links(&mut out, "Chassis", links.chassis.as_deref());
    push_reference_links(&mut out, "PCIeFunctions", links.pcie_functions.as_deref());
    push_reference_link(&mut out, "Switch", links.switch.as_ref());
    push_nav_links(&mut out, "Processors", links.processors.as_deref());
    push_nav_links(
        &mut out,
        "ConnectedPCIePorts",
        links.connected_pcie_ports.as_deref(),
    );
    out
}

fn firmware_links(
    links: Option<&nv_redfish::schema::software_inventory::Links>,
) -> BTreeMap<String, LinkData> {
    let mut out = BTreeMap::new();
    let Some(links) = links else {
        return out;
    };
    push_reference_links(&mut out, "StagedTargets", links.staged_targets.as_deref());
    push_reference_links(&mut out, "ActiveTargets", links.active_targets.as_deref());
    out
}

fn push_nav_links<T: EntityTypeRef>(
    out: &mut BTreeMap<String, LinkData>,
    relation: &str,
    links: Option<&[NavProperty<T>]>,
) {
    if let Some(links) = links {
        out.insert(
            relation.to_string(),
            LinkData::Multiple(
                links
                    .iter()
                    .filter_map(|link| normalize_link_target(&link.id().to_string()))
                    .collect(),
            ),
        );
    }
}

fn push_nav_link<T: EntityTypeRef>(
    out: &mut BTreeMap<String, LinkData>,
    relation: &str,
    link: Option<&NavProperty<T>>,
) {
    if let Some(target) = link.and_then(|link| normalize_link_target(&link.id().to_string())) {
        out.insert(relation.to_string(), LinkData::Single(target));
    }
}

fn reference_targets(links: &[ReferenceLeaf]) -> Vec<String> {
    links
        .iter()
        .filter_map(|link| normalize_link_target(&link.odata_id.to_string()))
        .collect()
}

fn push_reference_links(
    out: &mut BTreeMap<String, LinkData>,
    relation: &str,
    links: Option<&[ReferenceLeaf]>,
) {
    if let Some(links) = links {
        out.insert(
            relation.to_string(),
            LinkData::Multiple(reference_targets(links)),
        );
    }
}

fn push_reference_link(
    out: &mut BTreeMap<String, LinkData>,
    relation: &str,
    link: Option<&ReferenceLeaf>,
) {
    if let Some(target) = link.and_then(|link| normalize_link_target(&link.odata_id.to_string())) {
        out.insert(relation.to_string(), LinkData::Single(target));
    }
}

/// Strip authority/credentials and query parameters, but retain subobject identity.
fn normalize_link_target(odata_id: &str) -> Option<String> {
    let value = odata_id.trim();
    if value.is_empty() {
        return None;
    }
    if let Ok(url) = Url::parse(value)
        && url.has_host()
    {
        return Some(match url.fragment() {
            Some(fragment) => format!("{}#{fragment}", url.path()),
            None => url.path().to_string(),
        });
    }
    let (resource, fragment) = value
        .split_once('#')
        .map_or((value, None), |(resource, fragment)| {
            (resource, Some(fragment))
        });
    let path = resource.split('?').next().unwrap_or_default();
    if path.is_empty() && fragment.is_none() {
        return None;
    }
    Some(match fragment {
        Some(fragment) => format!("{path}#{fragment}"),
        None => path.to_string(),
    })
}

fn status_data(status: Option<&nv_redfish::schema::resource::Status>) -> Option<StatusData> {
    status.map(|status| StatusData {
        conditions: status
            .conditions
            .as_ref()
            .and_then(Option::as_ref)
            .map(|conditions| conditions.iter().map(map_condition).collect()),
        health: status.health.flatten().and_then(redfish_health_string),
        health_rollup: status
            .health_rollup
            .flatten()
            .and_then(redfish_health_string),
        state: status.state.flatten().and_then(redfish_state_string),
    })
}

fn map_condition(condition: &nv_redfish::schema::resource::Condition) -> ConditionData {
    ConditionData {
        timestamp: condition.timestamp.map(|timestamp| timestamp.to_string()),
        message_id: condition.message_id.clone(),
        message_args: condition.message_args.clone(),
        message: condition.message.clone(),
        severity: condition.severity.and_then(redfish_health_string),
        resolution: condition.resolution.clone(),
        condition_type: condition
            .condition_type
            .flatten()
            .and_then(redfish_condition_type_string),
        origin_of_condition: condition
            .origin_of_condition
            .as_ref()
            .map(|origin| origin.odata_id.to_string()),
    }
}

fn redfish_condition_type_string(
    condition_type: nv_redfish::schema::resource::ConditionType,
) -> Option<String> {
    match condition_type {
        nv_redfish::schema::resource::ConditionType::Alert => Some("Alert".to_string()),
        nv_redfish::schema::resource::ConditionType::Informational => {
            Some("Informational".to_string())
        }
        nv_redfish::schema::resource::ConditionType::Subsystem => Some("Subsystem".to_string()),
        nv_redfish::schema::resource::ConditionType::UnsupportedValue => None,
    }
}

fn redfish_health_string(health: nv_redfish::schema::resource::Health) -> Option<String> {
    match health {
        nv_redfish::schema::resource::Health::Ok => Some("OK".to_string()),
        nv_redfish::schema::resource::Health::Warning => Some("Warning".to_string()),
        nv_redfish::schema::resource::Health::Critical => Some("Critical".to_string()),
        nv_redfish::schema::resource::Health::UnsupportedValue => None,
    }
}

fn redfish_state_string(state: nv_redfish::schema::resource::State) -> Option<String> {
    match state {
        nv_redfish::schema::resource::State::Enabled => Some("Enabled".to_string()),
        nv_redfish::schema::resource::State::Disabled => Some("Disabled".to_string()),
        nv_redfish::schema::resource::State::StandbyOffline => Some("StandbyOffline".to_string()),
        nv_redfish::schema::resource::State::StandbySpare => Some("StandbySpare".to_string()),
        nv_redfish::schema::resource::State::InTest => Some("InTest".to_string()),
        nv_redfish::schema::resource::State::Starting => Some("Starting".to_string()),
        nv_redfish::schema::resource::State::Absent => Some("Absent".to_string()),
        nv_redfish::schema::resource::State::UnavailableOffline => {
            Some("UnavailableOffline".to_string())
        }
        nv_redfish::schema::resource::State::Deferring => Some("Deferring".to_string()),
        nv_redfish::schema::resource::State::Quiesced => Some("Quiesced".to_string()),
        nv_redfish::schema::resource::State::Updating => Some("Updating".to_string()),
        nv_redfish::schema::resource::State::Qualified => Some("Qualified".to_string()),
        nv_redfish::schema::resource::State::Degraded => Some("Degraded".to_string()),
        nv_redfish::schema::resource::State::UnsupportedValue => None,
    }
}

fn push_inventory_error(
    errors: &mut Vec<InventoryErrorData>,
    domain: &str,
    resource: Option<String>,
    error: impl std::fmt::Display,
) {
    errors.push(InventoryErrorData {
        domain: domain.to_string(),
        resource,
        message: error.to_string(),
    });
}

fn now_rfc3339() -> String {
    OffsetDateTime::now_utc()
        .format(&Rfc3339)
        .unwrap_or_else(|_| "1970-01-01T00:00:00Z".to_string())
}

fn bmc_url(address: &str) -> Result<Url, Box<dyn Error>> {
    let url: Url = if address.starts_with("http://") || address.starts_with("https://") {
        address.parse()?
    } else {
        format!("https://{address}").parse()?
    };
    if url.scheme() == "http" {
        tracing::warn!(
            address = %url,
            "BMC target is configured with http://; Redfish credentials and inventory will not use TLS"
        );
    }
    Ok(url)
}

#[cfg(test)]
mod tests {
    use std::convert::Infallible;

    use super::*;
    use bmc_dispatch::DispatcherConfig;
    use nv_redfish_bmc_mock::Expect;

    fn test_dispatcher() -> Dispatcher {
        Dispatcher::start(
            DispatcherConfig {
                global_max_in_flight: 1,
                bmc_max_in_flight: 1,
                bmc_requests_per_second: 100,
                bmc_burst: 100,
                workflow_queue_capacity: 8,
            },
            ["test".to_string()],
        )
        .expect("dispatcher")
    }

    #[tokio::test]
    async fn redfish_source_builds_client() {
        RedfishSource::new(true, Duration::from_secs(30), Arc::new(test_dispatcher()))
            .expect("source");
    }

    #[tokio::test]
    async fn redfish_source_rejects_zero_timeout() {
        let Err(error) = RedfishSource::new(true, Duration::ZERO, Arc::new(test_dispatcher()))
        else {
            panic!("zero timeout should fail");
        };

        assert!(!error.to_string().is_empty());
    }

    #[test]
    fn enum_values_keep_redfish_spelling() {
        use nv_redfish::schema::{
            chassis::ChassisType, manager::ManagerType, pcie_device::DeviceType,
            processor::ProcessorType, resource::PowerState,
        };
        assert_eq!(
            redfish_enum_string(ManagerType::Bmc).as_deref(),
            Some("BMC")
        );
        assert_eq!(
            redfish_enum_string(PowerState::PoweringOn).as_deref(),
            Some("PoweringOn")
        );
        assert_eq!(
            redfish_enum_string(ChassisType::RackMount).as_deref(),
            Some("RackMount")
        );
        assert_eq!(
            redfish_enum_string(ProcessorType::Cpu).as_deref(),
            Some("CPU")
        );
        assert_eq!(
            redfish_enum_string(DeviceType::SingleFunction).as_deref(),
            Some("SingleFunction")
        );
        assert_eq!(redfish_enum_string(ManagerType::UnsupportedValue), None);
    }

    #[test]
    fn bmc_url_adds_https_to_bare_address() {
        let url = bmc_url("10.0.0.1").expect("url");

        assert_eq!(url.as_str(), "https://10.0.0.1/");
        assert_eq!(url.port_or_known_default(), Some(443));
    }

    #[test]
    fn bmc_url_keeps_explicit_port() {
        let url = bmc_url("https://bmc.example.com:8443").expect("url");

        assert_eq!(url.as_str(), "https://bmc.example.com:8443/");
    }

    #[test]
    fn bmc_url_uses_scheme_default_port_for_scheme_only_url() {
        let url = bmc_url("http://bmc.example.com").expect("url");

        assert_eq!(url.as_str(), "http://bmc.example.com/");
        assert_eq!(url.port_or_known_default(), Some(80));
    }

    #[test]
    fn bmc_url_rejects_invalid_explicit_url() {
        let error = bmc_url("https://").expect_err("invalid url");

        assert!(!error.to_string().is_empty());
    }

    #[test]
    fn link_targets_preserve_fragments_and_relationship_shape() {
        assert_eq!(normalize_link_target("https://user:secret@bmc.example.com/redfish/v1/Systems/System?token=secret#/Processors/0").as_deref(),
            Some("/redfish/v1/Systems/System#/Processors/0"));
        assert_eq!(
            normalize_link_target("/redfish/v1/Systems/System?token=secret#/Processors/0")
                .as_deref(),
            Some("/redfish/v1/Systems/System#/Processors/0")
        );
        assert_eq!(
            normalize_link_target("#/Processors/0").as_deref(),
            Some("#/Processors/0")
        );
        assert_eq!(normalize_link_target("  "), None);
        let raw: nv_redfish::schema::chassis::Links = serde_json::from_value(serde_json::json!({
            "ContainedBy": {"@odata.id": "/redfish/v1/Chassis/Rack"},
            "Contains": [
                {"@odata.id": "/redfish/v1/Chassis/B"},
                {"@odata.id": "/redfish/v1/Chassis/A"},
                {"@odata.id": "/redfish/v1/Chassis/B"}
            ],
            "ManagedBy": []
        }))
        .unwrap();
        let links = chassis_links(Some(&raw));
        assert_eq!(
            links["ContainedBy"],
            LinkData::Single("/redfish/v1/Chassis/Rack".into())
        );
        assert_eq!(
            links["Contains"],
            LinkData::Multiple(vec![
                "/redfish/v1/Chassis/B".into(),
                "/redfish/v1/Chassis/A".into(),
                "/redfish/v1/Chassis/B".into()
            ])
        );
        assert_eq!(links["ManagedBy"], LinkData::Multiple(vec![]));
        assert!(!links.contains_key("ComputerSystems"));
    }

    #[test]
    fn pcie_traversal_policy_controls_selected_chassis() {
        assert!(should_collect_pcie_devices_for_chassis(
            &PcieTraversalPolicy::All,
            "BMC_0"
        ));
        assert!(should_collect_pcie_devices_for_chassis(
            &PcieTraversalPolicy::Viking,
            "HGX_GPU_SXM_0"
        ));
        assert!(should_collect_pcie_devices_for_chassis(
            &PcieTraversalPolicy::Viking,
            "HGX_NVSwitch_0"
        ));
        assert!(!should_collect_pcie_devices_for_chassis(
            &PcieTraversalPolicy::Viking,
            "HGX_GPU_PCIE_0"
        ));

        assert!(should_collect_pcie_devices_for_chassis(
            &PcieTraversalPolicy::SystemId("System".to_string()),
            "System"
        ));
        assert!(!should_collect_pcie_devices_for_chassis(
            &PcieTraversalPolicy::SystemId("System".to_string()),
            "BMC_0"
        ));

        assert!(!should_collect_pcie_devices_for_chassis(
            &PcieTraversalPolicy::None,
            "HGX_GPU_SXM_0"
        ));
    }

    #[test]
    fn pcie_traversal_policy_defaults_to_all_chassis() {
        let service_root = ServiceRootData {
            vendor: Some("NVIDIA".to_string()),
            product: Some("GB200 NVL".to_string()),
            ..ServiceRootData::default()
        };

        let policy = pcie_traversal_policy(&service_root, &[]);

        assert_eq!(policy, PcieTraversalPolicy::All);
    }

    #[test]
    fn ami_dgx_pcie_traversal_uses_viking_chassis_prefixes() {
        let service_root = ServiceRootData {
            vendor: Some("AMI".to_string()),
            ..ServiceRootData::default()
        };
        let systems = vec![SystemData {
            id: "DGX".to_string(),
            odata_id: None,
            uuid: None,
            manufacturer: None,
            model: None,
            sku: None,
            serial_number: None,
            bios_version: Some("1.0".to_string()),
            host_name: None,
            asset_tag: None,
            power_state: None,
            status: None,
            cpu_count: None,
            cpu_core_count: None,
            cpu_model: None,
            memory_gib: None,
            secure_boot_enabled: None,
            processors: Vec::new(),
            links: Default::default(),
        }];

        let policy = pcie_traversal_policy(&service_root, &systems);

        assert_eq!(policy, PcieTraversalPolicy::Viking);
    }

    #[test]
    fn selected_system_vendors_match_only_selected_system_chassis() {
        let service_root = ServiceRootData {
            vendor: Some("Dell".to_string()),
            ..ServiceRootData::default()
        };
        let systems = vec![SystemData {
            id: "System".to_string(),
            odata_id: None,
            uuid: None,
            manufacturer: None,
            model: None,
            sku: None,
            serial_number: None,
            bios_version: Some("1.0".to_string()),
            host_name: None,
            asset_tag: None,
            power_state: None,
            status: None,
            cpu_count: None,
            cpu_core_count: None,
            cpu_model: None,
            memory_gib: None,
            secure_boot_enabled: None,
            processors: Vec::new(),
            links: Default::default(),
        }];

        let policy = pcie_traversal_policy(&service_root, &systems);

        assert_eq!(policy, PcieTraversalPolicy::SystemId("System".to_string()));
    }

    #[tokio::test]
    async fn collect_report_from_mock_bmc_traverses_inventory() {
        type MockBmc = nv_redfish_bmc_mock::Bmc<Infallible>;

        let bmc = Arc::new(MockBmc::default());
        bmc.expect(Expect::get("/redfish/v1", SERVICE_ROOT));
        bmc.expect(Expect::get("/redfish/v1/Managers", MANAGERS_COLLECTION));
        bmc.expect(Expect::get("/redfish/v1/Managers/BMC_0", MANAGER));
        bmc.expect(Expect::get("/redfish/v1/Systems", SYSTEMS_COLLECTION));
        bmc.expect(Expect::get(
            "/redfish/v1/Systems/System",
            SYSTEM_WITH_PROCESSORS,
        ));
        bmc.expect(Expect::get(
            "/redfish/v1/Systems/System/Processors",
            PROCESSORS_COLLECTION,
        ));
        bmc.expect(Expect::get(
            "/redfish/v1/Systems/System/Processors/CPU1",
            PROCESSOR,
        ));
        bmc.expect(Expect::get("/redfish/v1/Chassis", CHASSIS_COLLECTION));
        bmc.expect(Expect::get("/redfish/v1/Chassis/HGX_GPU_SXM_0", CHASSIS));
        bmc.expect(Expect::get(
            "/redfish/v1/Chassis/HGX_GPU_SXM_0/PCIeDevices",
            PCIE_DEVICES_COLLECTION,
        ));
        bmc.expect(Expect::get(
            "/redfish/v1/Chassis/HGX_GPU_SXM_0/PCIeDevices/GPU1",
            PCIE_DEVICE,
        ));
        bmc.expect(Expect::get("/redfish/v1/UpdateService", UPDATE_SERVICE));
        bmc.expect(Expect::get(
            "/redfish/v1/UpdateService/FirmwareInventory",
            FIRMWARE_COLLECTION,
        ));
        bmc.expect(Expect::get(
            "/redfish/v1/UpdateService/FirmwareInventory/BMC",
            FIRMWARE,
        ));

        let root = ServiceRoot::new(Arc::clone(&bmc))
            .await
            .expect("service root wrapper");

        let report = collect_report_from_root(&root).await.expect("report");

        assert_eq!(
            report.service_root.redfish_version.as_deref(),
            Some("1.15.1")
        );
        assert_eq!(report.service_root.uuid.as_deref(), Some(SERVICE_UUID));
        assert_eq!(report.service_root.vendor.as_deref(), Some("AMI"));
        assert_eq!(report.service_root.product.as_deref(), Some("Mock BMC"));

        assert_eq!(report.managers.len(), 1);
        assert_eq!(report.managers[0].id, "BMC_0");
        assert_eq!(
            report.managers[0].firmware_version.as_deref(),
            Some("3.19.00")
        );
        assert_eq!(
            report.managers[0].links,
            BTreeMap::from([(
                "ManagerForServers".to_string(),
                LinkData::Multiple(vec!["/redfish/v1/Systems/System#item".into()])
            )])
        );

        assert_eq!(report.systems.len(), 1);
        assert_eq!(report.systems[0].id, "DGX");
        assert_eq!(report.systems[0].uuid.as_deref(), Some(SYSTEM_UUID));
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
        assert_eq!(report.systems[0].cpu_count, Some(1));
        assert_eq!(report.systems[0].cpu_core_count, Some(72));
        assert_eq!(report.systems[0].cpu_model.as_deref(), Some("Grace"));
        assert_eq!(report.systems[0].memory_gib, Some(480.0));
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
                    LinkData::Multiple(vec!["/redfish/v1/Chassis/HGX_GPU_SXM_0".into()])
                ),
                (
                    "ManagedBy".into(),
                    LinkData::Multiple(vec!["/redfish/v1/Managers/BMC_0".into()])
                )
            ])
        );
        assert_eq!(
            processor.links,
            BTreeMap::from([(
                "Chassis".to_string(),
                LinkData::Single("/redfish/v1/Chassis/HGX_GPU_SXM_0".into())
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
            condition.message_args.as_deref(),
            Some(
                [
                    "CPU_1 Firmware".to_string(),
                    "EC_STRAP_MISMATCH".to_string()
                ]
                .as_slice()
            )
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
            condition.origin_of_condition.as_deref(),
            Some("/redfish/v1/Chassis/HGX_CPU_1")
        );
        let location = report.chassis[0]
            .location
            .as_ref()
            .expect("chassis location");
        assert_eq!(location.service_label.as_deref(), Some("Bay 1"));
        assert_eq!(location.rack.as_deref(), Some("R12"));
        assert_eq!(location.row.as_deref(), Some("A"));
        assert_eq!(location.rack_offset, Some(7));
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
            Some(vec!["/redfish/v1/Managers/BMC_0#/Firmware".into()])
        );
    }

    #[tokio::test]
    async fn secure_boot_collection_failure_records_system_id_resource() {
        type MockBmc = nv_redfish_bmc_mock::Bmc<Infallible>;

        let bmc = Arc::new(MockBmc::default());
        bmc.expect(Expect::get("/redfish/v1", SERVICE_ROOT));
        bmc.expect(Expect::get("/redfish/v1/Managers", MANAGERS_COLLECTION));
        bmc.expect(Expect::get("/redfish/v1/Managers/BMC_0", MANAGER));
        bmc.expect(Expect::get("/redfish/v1/Systems", SYSTEMS_COLLECTION));
        bmc.expect(Expect::get(
            "/redfish/v1/Systems/System",
            SYSTEM_WITH_SECURE_BOOT,
        ));
        bmc.expect(Expect::get(
            "/redfish/v1/Systems/System/SecureBoot",
            MALFORMED_SECURE_BOOT,
        ));
        bmc.expect(Expect::get("/redfish/v1/Chassis", CHASSIS_COLLECTION));
        bmc.expect(Expect::get("/redfish/v1/Chassis/HGX_GPU_SXM_0", CHASSIS));
        bmc.expect(Expect::get(
            "/redfish/v1/Chassis/HGX_GPU_SXM_0/PCIeDevices",
            PCIE_DEVICES_COLLECTION,
        ));
        bmc.expect(Expect::get(
            "/redfish/v1/Chassis/HGX_GPU_SXM_0/PCIeDevices/GPU1",
            PCIE_DEVICE,
        ));
        bmc.expect(Expect::get("/redfish/v1/UpdateService", UPDATE_SERVICE));
        bmc.expect(Expect::get(
            "/redfish/v1/UpdateService/FirmwareInventory",
            FIRMWARE_COLLECTION,
        ));
        bmc.expect(Expect::get(
            "/redfish/v1/UpdateService/FirmwareInventory/BMC",
            FIRMWARE,
        ));

        let root = ServiceRoot::new(Arc::clone(&bmc))
            .await
            .expect("service root wrapper");

        let report = collect_report_from_root(&root).await.expect("report");

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
        type MockBmc = nv_redfish_bmc_mock::Bmc<Infallible>;

        let bmc = Arc::new(MockBmc::default());
        bmc.expect(Expect::get("/redfish/v1", SERVICE_ROOT));
        bmc.expect(Expect::get("/redfish/v1/Managers", MANAGERS_COLLECTION));
        bmc.expect(Expect::get("/redfish/v1/Managers/BMC_0", MANAGER));
        bmc.expect(Expect::get("/redfish/v1/Systems", SYSTEMS_COLLECTION));
        bmc.expect(Expect::get("/redfish/v1/Systems/System", SYSTEM));
        bmc.expect(Expect::get("/redfish/v1/Chassis", CHASSIS_COLLECTION));
        bmc.expect(Expect::get("/redfish/v1/Chassis/HGX_GPU_SXM_0", CHASSIS));
        bmc.expect(Expect::get(
            "/redfish/v1/Chassis/HGX_GPU_SXM_0/PCIeDevices",
            MALFORMED_PCIE_DEVICES_COLLECTION,
        ));
        bmc.expect(Expect::get("/redfish/v1/UpdateService", UPDATE_SERVICE));
        bmc.expect(Expect::get(
            "/redfish/v1/UpdateService/FirmwareInventory",
            FIRMWARE_COLLECTION,
        ));
        bmc.expect(Expect::get(
            "/redfish/v1/UpdateService/FirmwareInventory/BMC",
            FIRMWARE,
        ));

        let root = ServiceRoot::new(Arc::clone(&bmc))
            .await
            .expect("service root wrapper");

        let report = collect_report_from_root(&root).await.expect("report");

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
        type MockBmc = nv_redfish_bmc_mock::Bmc<Infallible>;

        let bmc = Arc::new(MockBmc::default());
        bmc.expect(Expect::get("/redfish/v1", SERVICE_ROOT));
        bmc.expect(Expect::get("/redfish/v1/Managers", MANAGERS_COLLECTION));
        bmc.expect(Expect::get("/redfish/v1/Managers/BMC_0", MALFORMED_MANAGER));
        bmc.expect(Expect::get("/redfish/v1/Systems", SYSTEMS_COLLECTION));
        bmc.expect(Expect::get("/redfish/v1/Systems/System", SYSTEM));
        bmc.expect(Expect::get("/redfish/v1/Chassis", CHASSIS_COLLECTION));
        bmc.expect(Expect::get("/redfish/v1/Chassis/HGX_GPU_SXM_0", CHASSIS));
        bmc.expect(Expect::get(
            "/redfish/v1/Chassis/HGX_GPU_SXM_0/PCIeDevices",
            PCIE_DEVICES_COLLECTION,
        ));
        bmc.expect(Expect::get(
            "/redfish/v1/Chassis/HGX_GPU_SXM_0/PCIeDevices/GPU1",
            PCIE_DEVICE,
        ));
        bmc.expect(Expect::get("/redfish/v1/UpdateService", UPDATE_SERVICE));
        bmc.expect(Expect::get(
            "/redfish/v1/UpdateService/FirmwareInventory",
            FIRMWARE_COLLECTION,
        ));
        bmc.expect(Expect::get(
            "/redfish/v1/UpdateService/FirmwareInventory/BMC",
            FIRMWARE,
        ));

        let root = ServiceRoot::new(Arc::clone(&bmc))
            .await
            .expect("service root wrapper");

        let report = collect_report_from_root(&root).await.expect("report");

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
        type MockBmc = nv_redfish_bmc_mock::Bmc<Infallible>;

        let bmc = Arc::new(MockBmc::default());
        bmc.expect(Expect::get("/redfish/v1", SERVICE_ROOT));
        bmc.expect(Expect::get("/redfish/v1/Managers", MANAGERS_COLLECTION));
        bmc.expect(Expect::get("/redfish/v1/Managers/BMC_0", MANAGER));
        bmc.expect(Expect::get("/redfish/v1/Systems", SYSTEMS_COLLECTION));
        bmc.expect(Expect::get("/redfish/v1/Systems/System", SYSTEM));
        bmc.expect(Expect::get("/redfish/v1/Chassis", CHASSIS_COLLECTION));
        bmc.expect(Expect::get("/redfish/v1/Chassis/HGX_GPU_SXM_0", CHASSIS));
        bmc.expect(Expect::get(
            "/redfish/v1/Chassis/HGX_GPU_SXM_0/PCIeDevices",
            PCIE_DEVICES_COLLECTION,
        ));
        bmc.expect(Expect::get(
            "/redfish/v1/Chassis/HGX_GPU_SXM_0/PCIeDevices/GPU1",
            PCIE_DEVICE,
        ));
        bmc.expect(Expect::get("/redfish/v1/UpdateService", UPDATE_SERVICE));
        bmc.expect(Expect::get(
            "/redfish/v1/UpdateService/FirmwareInventory",
            FIRMWARE_COLLECTION,
        ));
        bmc.expect(Expect::get(
            "/redfish/v1/UpdateService/FirmwareInventory/BMC",
            MALFORMED_FIRMWARE,
        ));

        let root = ServiceRoot::new(Arc::clone(&bmc))
            .await
            .expect("service root wrapper");

        let report = collect_report_from_root(&root).await.expect("report");

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
