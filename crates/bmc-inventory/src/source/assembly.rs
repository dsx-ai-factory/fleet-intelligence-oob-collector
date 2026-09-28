use bmc_error::SanitizedErrorMessage;

use super::RedfishReport;
use crate::config::Target;
use crate::models::{
    INVENTORY_SCHEMA_VERSION, InventorySourceInfo, InventoryUuid, LatencyMeasurement,
    NodeInventory, SystemInventory,
};

const DERIVED_NODE_UUID_KEY_PREFIX: &str = "fleet-intelligence-oob:redfish:";

pub(super) fn assemble(
    target: Target,
    collected_at: String,
    host_name: Option<String>,
    result: Result<RedfishReport, String>,
    latency: Vec<LatencyMeasurement>,
) -> NodeInventory {
    let mut inventory = NodeInventory {
        schema_version: INVENTORY_SCHEMA_VERSION.to_string(),
        collected_at,
        enrolled_at: None,
        kind: target.kind,
        name: target.name,
        compute_zone: target.compute_zone,
        node_group: target.node_group,
        uuid: None,
        source: InventorySourceInfo {
            source_type: "redfish".to_string(),
            mac: target.mac,
            address: target.address,
            host_name,
            redfish_version: None,
            service_uuid: None,
            vendor: None,
        },
        primary_system_id: None,
        managers: Vec::new(),
        systems: Vec::new(),
        chassis: Vec::new(),
        firmware: Vec::new(),
        fabrics: Vec::new(),
        domain_errors: Vec::new(),
        target_error: None,
        latency,
    };

    match result {
        Ok(report) => {
            inventory.source.redfish_version = report.service_root.redfish_version;
            inventory.source.service_uuid = report.service_root.uuid;
            inventory.source.vendor = report.service_root.vendor.or(report.service_root.product);

            let selected = select_primary_system(&report.systems);
            let node_uuid = selected
                .and_then(|system| system.uuid.clone())
                .unwrap_or_else(|| derived_node_uuid(&inventory.source));
            inventory.primary_system_id = selected.map(|system| system.id.clone());
            inventory.uuid = Some(node_uuid.clone());
            inventory.managers = report.managers;
            inventory.systems = report.systems;
            for system in inventory
                .systems
                .iter_mut()
                .filter(|system| Some(&system.id) == inventory.primary_system_id.as_ref())
            {
                system.uuid = Some(node_uuid.clone());
            }
            inventory.chassis = report.chassis;
            inventory.firmware = report.firmware;
            inventory.fabrics = report.fabrics;
            inventory.domain_errors = report.errors;
        }
        Err(error) => inventory.target_error = Some(SanitizedErrorMessage::new(error)),
    }
    inventory
}

fn select_primary_system(systems: &[SystemInventory]) -> Option<&SystemInventory> {
    systems
        .iter()
        .find(|system| {
            system
                .bios_version
                .as_deref()
                .is_some_and(|value| !value.trim().is_empty())
        })
        .or_else(|| systems.first())
}

fn derived_node_uuid(source: &InventorySourceInfo) -> InventoryUuid {
    let mut key = String::from(DERIVED_NODE_UUID_KEY_PREFIX);
    match normalized_service_uuid(source.service_uuid.as_deref()) {
        Some(service_uuid) => {
            key.push_str("service:");
            key.push_str(&service_uuid);
        }
        None => {
            key.push_str("address:");
            key.push_str(source.address.trim());
        }
    }
    InventoryUuid::from_stable_name(key.as_bytes())
}

fn normalized_service_uuid(service_uuid: Option<&str>) -> Option<String> {
    let service_uuid = service_uuid?.trim();
    if service_uuid.is_empty() {
        return None;
    }
    InventoryUuid::parse(service_uuid)
        .map(|uuid| uuid.to_string())
        .or_else(|| Some(service_uuid.to_string()))
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeMap;

    use super::*;
    use crate::models::{DomainError, TargetKind};
    use crate::source::ServiceRootData;

    fn target() -> Target {
        Target {
            kind: TargetKind::Compute,
            name: Some("node-a".into()),
            compute_zone: None,
            node_group: None,
            ip: "192.0.2.10".parse().unwrap(),
            port: None,
            mac: "00:11:22:33:44:55".into(),
            address: "192.0.2.10".into(),
            username: "user".into(),
            password: "password".into(),
            labels: BTreeMap::new(),
        }
    }

    fn system(id: &str, bios: Option<&str>, uuid: Option<&str>) -> SystemInventory {
        SystemInventory {
            name: id.into(),
            description: None,
            id: id.into(),
            odata_id: None,
            uuid: uuid.and_then(InventoryUuid::parse),
            manufacturer: None,
            model: None,
            sku: None,
            serial_number: None,
            bios_version: bios.map(str::to_string),
            host_name: None,
            asset_tag: None,
            power_state: None,
            status: None,
            processor_summary: None,
            memory_summary: None,
            secure_boot_enabled: None,
            processors: Vec::new(),
            links: BTreeMap::new(),
        }
    }

    fn report(systems: Vec<SystemInventory>) -> RedfishReport {
        RedfishReport {
            service_root: ServiceRootData {
                redfish_version: Some("1.15.1".into()),
                uuid: Some("870155DD-D474-ACA8-11EE-6E809AF87C0C".into()),
                vendor: Some("AMI".into()),
                product: Some("Mock BMC".into()),
            },
            managers: Vec::new(),
            systems,
            chassis: Vec::new(),
            firmware: Vec::new(),
            fabrics: Vec::new(),
            errors: Vec::new(),
        }
    }

    #[test]
    fn selects_bios_system_and_keeps_other_system_identity() {
        let auxiliary_uuid = "11111111-2222-3333-4444-555555555555";
        let host_uuid = "aaaaaaaa-bbbb-cccc-dddd-eeeeeeeeeeee";
        let inventory = assemble(
            target(),
            "2026-01-01T00:00:00Z".into(),
            Some("bmc.example.com".into()),
            Ok(report(vec![
                system("Aux", None, Some(auxiliary_uuid)),
                system("Host", Some("3A21"), Some(host_uuid)),
            ])),
            Vec::new(),
        );

        assert_eq!(inventory.primary_system_id.as_deref(), Some("Host"));
        assert_eq!(inventory.uuid.as_ref().unwrap().to_string(), host_uuid);
        assert_eq!(
            inventory.systems[0].uuid.as_ref().unwrap().to_string(),
            auxiliary_uuid
        );
        assert_eq!(inventory.systems[1].uuid, inventory.uuid);
        assert_eq!(
            inventory.source.host_name.as_deref(),
            Some("bmc.example.com")
        );
        assert_eq!(inventory.source.vendor.as_deref(), Some("AMI"));
    }

    #[test]
    fn missing_selected_uuid_uses_normalized_service_uuid() {
        let inventory = assemble(
            target(),
            "2026-01-01T00:00:00Z".into(),
            None,
            Ok(report(vec![system("Host", Some("3A21"), None)])),
            Vec::new(),
        );
        let expected = InventoryUuid::from_stable_name(
            b"fleet-intelligence-oob:redfish:service:870155dd-d474-aca8-11ee-6e809af87c0c",
        );
        assert_eq!(inventory.uuid, Some(expected));
        assert_eq!(inventory.systems[0].uuid, inventory.uuid);
    }

    #[test]
    fn partial_and_target_failures_keep_distinct_output() {
        let mut partial = report(Vec::new());
        partial.errors.push(DomainError {
            domain: "firmware".into(),
            resource: None,
            message: SanitizedErrorMessage::new("firmware unavailable"),
        });
        let inventory = assemble(target(), "time".into(), None, Ok(partial), Vec::new());
        assert_eq!(inventory.domain_errors.len(), 1);
        assert!(inventory.target_error.is_none());
        assert!(inventory.uuid.is_some());

        let failed = assemble(
            target(),
            "time".into(),
            None,
            Err("request failed at https://user:secret@bmc.example.com/redfish/v1".into()),
            Vec::new(),
        );
        assert!(failed.uuid.is_none());
        assert!(failed.systems.is_empty());
        assert!(failed.domain_errors.is_empty());
        assert_eq!(
            failed.target_error.unwrap().as_ref(),
            "request failed at [URL_REDACTED]"
        );
    }
}
