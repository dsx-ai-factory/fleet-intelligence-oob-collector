use std::collections::BTreeMap;

use bmc_error::SanitizedUriReference;
use nv_redfish::core::{EntityTypeRef, NavProperty, ReferenceLeaf};
use url::Url;

use crate::models::{InventoryLink, InventoryLinkReference};

pub(super) fn manager_links(
    links: Option<&nv_redfish::schema::manager::Links>,
) -> BTreeMap<String, InventoryLink> {
    let mut out = BTreeMap::new();
    let Some(links) = links else {
        return out;
    };
    push_links(
        &mut out,
        "ManagerForServers",
        links.manager_for_servers.as_deref(),
    );
    push_links(
        &mut out,
        "ManagerForChassis",
        links.manager_for_chassis.as_deref(),
    );
    push_link(
        &mut out,
        "ManagerInChassis",
        links.manager_in_chassis.as_ref(),
    );
    push_links(
        &mut out,
        "ManagerForSwitches",
        links.manager_for_switches.as_deref(),
    );
    push_link(
        &mut out,
        "ActiveSoftwareImage",
        links.active_software_image.as_ref(),
    );
    push_links(&mut out, "SoftwareImages", links.software_images.as_deref());
    push_links(&mut out, "ManagedBy", links.managed_by.as_deref());
    push_links(
        &mut out,
        "ManagerForManagers",
        links.manager_for_managers.as_deref(),
    );
    push_link(
        &mut out,
        "SelectedNetworkPort",
        links.selected_network_port.as_ref(),
    );
    push_links(
        &mut out,
        "ManagerForFabrics",
        links.manager_for_fabrics.as_deref(),
    );
    out
}

pub(super) fn system_links(
    links: Option<&nv_redfish::schema::computer_system::Links>,
) -> BTreeMap<String, InventoryLink> {
    let mut out = BTreeMap::new();
    let Some(links) = links else {
        return out;
    };
    push_links(&mut out, "Chassis", links.chassis.as_deref());
    push_links(&mut out, "ManagedBy", links.managed_by.as_deref());
    push_links(&mut out, "PoweredBy", links.powered_by.as_deref());
    push_links(&mut out, "CooledBy", links.cooled_by.as_deref());
    push_links(&mut out, "Endpoints", links.endpoints.as_deref());
    push_links(&mut out, "ResourceBlocks", links.resource_blocks.as_deref());
    push_links(
        &mut out,
        "ConsumingComputerSystems",
        links.consuming_computer_systems.as_deref(),
    );
    push_links(
        &mut out,
        "SupplyingComputerSystems",
        links.supplying_computer_systems.as_deref(),
    );
    push_links(
        &mut out,
        "OffloadedNetworkDeviceFunctions",
        links.offloaded_network_device_functions.as_deref(),
    );
    push_links(
        &mut out,
        "TrustedComponents",
        links.trusted_components.as_deref(),
    );
    push_link(
        &mut out,
        "HostingComputerSystem",
        links.hosting_computer_system.as_ref(),
    );
    push_links(
        &mut out,
        "VirtualMachines",
        links.virtual_machines.as_deref(),
    );
    out
}

pub(super) fn chassis_links(
    links: Option<&nv_redfish::schema::chassis::Links>,
) -> BTreeMap<String, InventoryLink> {
    let mut out = BTreeMap::new();
    let Some(links) = links else {
        return out;
    };
    push_links(
        &mut out,
        "ComputerSystems",
        links.computer_systems.as_deref(),
    );
    push_links(&mut out, "ManagedBy", links.managed_by.as_deref());
    push_link(&mut out, "ContainedBy", links.contained_by.as_ref());
    push_links(&mut out, "Contains", links.contains.as_deref());
    push_links(&mut out, "PoweredBy", links.powered_by.as_deref());
    push_links(&mut out, "CooledBy", links.cooled_by.as_deref());
    push_links(
        &mut out,
        "ManagersInChassis",
        links.managers_in_chassis.as_deref(),
    );
    push_links(&mut out, "Drives", links.drives.as_deref());
    push_links(&mut out, "Storage", links.storage.as_deref());
    push_links(&mut out, "PCIeDevices", links.pcie_devices.as_deref());
    push_links(&mut out, "ResourceBlocks", links.resource_blocks.as_deref());
    push_links(&mut out, "Switches", links.switches.as_deref());
    push_links(&mut out, "Processors", links.processors.as_deref());
    push_link(&mut out, "Facility", links.facility.as_ref());
    push_links(&mut out, "Cables", links.cables.as_deref());
    push_links(&mut out, "PowerOutlets", links.power_outlets.as_deref());
    push_links(&mut out, "PowerSupplies", links.power_supplies.as_deref());
    push_links(&mut out, "Fans", links.fans.as_deref());
    push_link(
        &mut out,
        "PowerDistribution",
        links.power_distribution.as_ref(),
    );
    push_links(&mut out, "CoolingUnits", links.cooling_units.as_deref());
    push_links(
        &mut out,
        "ConnectedCoolingLoops",
        links.connected_cooling_loops.as_deref(),
    );
    push_links(
        &mut out,
        "AutomationNodes",
        links.automation_nodes.as_deref(),
    );
    out
}

pub(super) fn processor_links(
    links: Option<&nv_redfish::schema::processor::Links>,
) -> BTreeMap<String, InventoryLink> {
    let mut out = BTreeMap::new();
    let Some(links) = links else {
        return out;
    };
    push_link(&mut out, "Chassis", links.chassis.as_ref());
    push_links(&mut out, "Endpoints", links.endpoints.as_deref());
    push_links(
        &mut out,
        "ConnectedProcessors",
        links.connected_processors.as_deref(),
    );
    push_link(&mut out, "PCIeDevice", links.pcie_device.as_ref());
    push_links(&mut out, "PCIeFunctions", links.pcie_functions.as_deref());
    push_links(&mut out, "Memory", links.memory.as_deref());
    push_link(
        &mut out,
        "GraphicsController",
        links.graphics_controller.as_ref(),
    );
    push_links(
        &mut out,
        "NetworkDeviceFunctions",
        links.network_device_functions.as_deref(),
    );
    push_links(&mut out, "FabricAdapters", links.fabric_adapters.as_deref());
    out
}

pub(super) fn pcie_device_links(
    links: Option<&nv_redfish::schema::pcie_device::Links>,
) -> BTreeMap<String, InventoryLink> {
    let mut out = BTreeMap::new();
    let Some(links) = links else {
        return out;
    };
    push_links(&mut out, "Chassis", links.chassis.as_deref());
    push_links(&mut out, "PCIeFunctions", links.pcie_functions.as_deref());
    push_link(&mut out, "Switch", links.switch.as_ref());
    push_links(&mut out, "Processors", links.processors.as_deref());
    push_links(
        &mut out,
        "ConnectedPCIePorts",
        links.connected_pcie_ports.as_deref(),
    );
    out
}

pub(super) fn firmware_links(
    links: Option<&nv_redfish::schema::software_inventory::Links>,
) -> BTreeMap<String, InventoryLink> {
    let mut out = BTreeMap::new();
    let Some(links) = links else {
        return out;
    };
    push_links(&mut out, "StagedTargets", links.staged_targets.as_deref());
    push_links(&mut out, "ActiveTargets", links.active_targets.as_deref());
    out
}

/// A Redfish link target. nv-redfish types a link as `NavProperty` when the
/// target schema is compiled in and as a plain `ReferenceLeaf` otherwise, so
/// the link form depends on enabled features; both carry an `@odata.id`.
pub(super) trait LinkTarget {
    fn target_id(&self) -> String;
}

impl<T: EntityTypeRef> LinkTarget for NavProperty<T> {
    fn target_id(&self) -> String {
        self.id().to_string()
    }
}

impl LinkTarget for ReferenceLeaf {
    fn target_id(&self) -> String {
        self.odata_id.to_string()
    }
}

fn push_links<L: LinkTarget>(
    out: &mut BTreeMap<String, InventoryLink>,
    relation: &str,
    links: Option<&[L]>,
) {
    if let Some(links) = links {
        out.insert(relation.to_string(), link_targets(links));
    }
}

fn push_link<L: LinkTarget>(
    out: &mut BTreeMap<String, InventoryLink>,
    relation: &str,
    link: Option<&L>,
) {
    if let Some(target) = link.and_then(|link| normalize_link_target(&link.target_id())) {
        out.insert(relation.to_string(), vec![link_reference(target)]);
    }
}

pub(super) fn link_reference(odata_id: String) -> InventoryLinkReference {
    InventoryLinkReference {
        odata_id: SanitizedUriReference::new(odata_id),
    }
}

pub(super) fn link_targets<L: LinkTarget>(links: &[L]) -> InventoryLink {
    links
        .iter()
        .filter_map(|link| normalize_link_target(&link.target_id()))
        .map(link_reference)
        .collect()
}

/// Strip authority/credentials and query parameters, but retain subobject identity.
fn normalize_link_target(odata_id: &str) -> Option<String> {
    let value = odata_id.trim();
    if value.is_empty() {
        return None;
    }
    if let Some(reference) = value.strip_prefix("//") {
        let url = Url::parse(&format!("https://{reference}")).ok()?;
        if !url.has_host() {
            return None;
        }
        return Some(match url.fragment() {
            Some(fragment) => format!("{}#{fragment}", url.path()),
            None => url.path().to_string(),
        });
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
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn typed_and_plain_links_produce_identical_targets() {
        let target = serde_json::json!({
            "@odata.id": "https://user:secret@bmc.example.com/redfish/v1/Chassis/A?token=secret#/Oem"
        });
        let typed: NavProperty<nv_redfish::schema::chassis::Chassis> =
            serde_json::from_value(target.clone()).expect("typed link");
        let plain: ReferenceLeaf = serde_json::from_value(target).expect("plain link");

        let expected = vec![link_reference("/redfish/v1/Chassis/A#/Oem".to_string())];
        assert_eq!(link_targets(&[typed]), expected);
        assert_eq!(link_targets(&[plain]), expected);
    }

    #[test]
    fn link_targets_preserve_fragments_and_normalize_relationships_to_arrays() {
        assert_eq!(normalize_link_target("https://user:secret@bmc.example.com/redfish/v1/Systems/System?token=secret#/Processors/0").as_deref(),
            Some("/redfish/v1/Systems/System#/Processors/0"));
        assert_eq!(
            normalize_link_target("/redfish/v1/Systems/System?token=secret#/Processors/0")
                .as_deref(),
            Some("/redfish/v1/Systems/System#/Processors/0")
        );
        assert_eq!(
            normalize_link_target(
                "//user:secret@bmc.example.com/redfish/v1/Systems/System?token=secret#/Processors/0"
            )
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
            vec![link_reference("/redfish/v1/Chassis/Rack".to_string())]
        );
        assert_eq!(
            links["Contains"],
            vec![
                link_reference("/redfish/v1/Chassis/B".to_string()),
                link_reference("/redfish/v1/Chassis/A".to_string()),
                link_reference("/redfish/v1/Chassis/B".to_string())
            ]
        );
        assert_eq!(links["ManagedBy"], Vec::<InventoryLinkReference>::new());
        assert!(!links.contains_key("ComputerSystems"));
    }

    #[test]
    fn source_sanitizes_link() {
        let link = link_reference(
            "https://user:secret@bmc.example.com/redfish/v1/Systems/A?token=secret#item"
                .to_string(),
        );
        assert_eq!(link.odata_id.as_ref(), "/redfish/v1/Systems/A#item");
    }
}
