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

use bmc_error::SanitizedErrorMessage;

use super::RedfishReport;
use crate::config::Target;
use crate::models::{
    INVENTORY_SCHEMA_VERSION, InventorySourceInfo, LatencyMeasurement, NodeInventory,
};

pub(super) fn assemble(
    target: Target,
    collected_at: String,
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
        source: InventorySourceInfo {
            source_type: "redfish".to_string(),
            mac: target.mac.to_string(),
            address: target.address,
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
        target_error: None,
        latency,
    };

    match result {
        Ok(report) => {
            inventory.source.redfish_version = report.service_root.redfish_version;
            inventory.source.service_uuid = report.service_root.uuid;
            inventory.source.vendor = report.service_root.vendor.or(report.service_root.product);

            inventory.managers = report.managers;
            inventory.systems = report.systems;
            inventory.chassis = report.chassis;
            inventory.firmware = report.firmware;
            inventory.fabrics = report.fabrics;
            inventory.domain_errors = report.errors;
        }
        Err(error) => inventory.target_error = Some(SanitizedErrorMessage::new(error)),
    }
    inventory
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeMap;

    use super::*;
    use crate::models::{DomainError, InventoryUuid, SystemInventory, TargetKind};
    use crate::source::ServiceRootData;

    fn target() -> Target {
        Target {
            kind: TargetKind::Compute,
            name: Some("node-a".into()),
            compute_zone: None,
            node_group: None,
            ip: "192.0.2.10".parse().unwrap(),
            port: None,
            mac: "00:11:22:33:44:55".parse().unwrap(),
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
    fn preserves_all_reported_system_identities() {
        let auxiliary_uuid = "11111111-2222-3333-4444-555555555555";
        let host_uuid = "aaaaaaaa-bbbb-cccc-dddd-eeeeeeeeeeee";
        let inventory = assemble(
            target(),
            "2026-01-01T00:00:00Z".into(),
            Ok(report(vec![
                system("Aux", None, Some(auxiliary_uuid)),
                system("Host", Some("3A21"), Some(host_uuid)),
            ])),
            Vec::new(),
        );

        assert_eq!(
            inventory.systems[0].uuid.as_ref().unwrap().to_string(),
            auxiliary_uuid
        );
        assert_eq!(
            inventory.systems[1].uuid.as_ref().unwrap().to_string(),
            host_uuid
        );
        assert_eq!(inventory.source.vendor.as_deref(), Some("AMI"));
    }

    #[test]
    fn system_order_does_not_select_or_generate_identity() {
        for reversed in [false, true] {
            let mut systems = vec![
                system(
                    "System_0",
                    None,
                    Some("aaaaaaaa-bbbb-cccc-dddd-eeeeeeeeeeee"),
                ),
                system("HGX_Baseboard_0", None, None),
            ];
            if reversed {
                systems.reverse();
            }
            let inventory = assemble(target(), "time".into(), Ok(report(systems)), Vec::new());
            let host = inventory
                .systems
                .iter()
                .find(|system| system.id == "System_0")
                .unwrap();
            let baseboard = inventory
                .systems
                .iter()
                .find(|system| system.id == "HGX_Baseboard_0")
                .unwrap();
            assert_eq!(
                host.uuid.as_ref().unwrap().to_string(),
                "aaaaaaaa-bbbb-cccc-dddd-eeeeeeeeeeee"
            );
            assert!(baseboard.uuid.is_none());
        }
    }

    #[test]
    fn missing_system_uuid_stays_missing() {
        let inventory = assemble(
            target(),
            "2026-01-01T00:00:00Z".into(),
            Ok(report(vec![system("Host", Some("3A21"), None)])),
            Vec::new(),
        );
        assert!(inventory.systems[0].uuid.is_none());
    }

    #[test]
    fn partial_and_target_failures_keep_distinct_output() {
        let mut partial = report(Vec::new());
        partial.errors.push(DomainError {
            domain: "firmware".into(),
            resource: None,
            message: SanitizedErrorMessage::new("firmware unavailable"),
        });
        let inventory = assemble(target(), "time".into(), Ok(partial), Vec::new());
        assert_eq!(inventory.domain_errors.len(), 1);
        assert!(inventory.target_error.is_none());
        assert_eq!(inventory.source.mac, target().mac.to_string());

        let failed = assemble(
            target(),
            "time".into(),
            Err("request failed at https://user:secret@bmc.example.com/redfish/v1".into()),
            Vec::new(),
        );
        assert_eq!(failed.source.mac, target().mac.to_string());
        assert!(failed.systems.is_empty());
        assert!(failed.domain_errors.is_empty());
        assert_eq!(
            failed.target_error.unwrap().as_ref(),
            "request failed at [URL_REDACTED]"
        );
    }
}
