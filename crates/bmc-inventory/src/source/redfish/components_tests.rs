// Copyright (c) 2026, NVIDIA CORPORATION & AFFILIATES. All rights reserved.
// SPDX-License-Identifier: Apache-2.0

use super::resources::collect_report_from_root;
use nv_redfish::ServiceRoot;
use nv_redfish_bmc_mock::{Bmc, Expect};
use serde_json::{Value, json};
use std::{convert::Infallible, sync::Arc};

fn resource(path: &str, kind: &str, id: &str, extra: Value) -> Value {
    let mut value = json!({"@odata.id": path, "@odata.type": kind, "Id": id, "Name": id});
    value
        .as_object_mut()
        .unwrap()
        .extend(extra.as_object().unwrap().clone());
    value
}

async fn collect(broken: bool, legacy_power: bool) -> super::super::RedfishReport {
    let bmc = Arc::new(Bmc::<Infallible>::default());
    let system = "/redfish/v1/Systems/Tray";
    let chassis = "/redfish/v1/Chassis/Tray";
    let memory = format!("{system}/Memory/DIMM0");
    let storage = format!("{system}/Storage/NVMe");
    let drive = format!("{storage}/Drives/SSD0");
    let ps = format!("{chassis}/PowerSubsystem");
    let psu = format!("{ps}/PowerSupplies/PSU0");
    let mut routes = vec![
        (
            "/redfish/v1".into(),
            resource(
                "/redfish/v1",
                "#ServiceRoot.v1_15_0.ServiceRoot",
                "Root",
                json!({
                    "RedfishVersion":"1.15.0", "Links":{"Sessions":{"@odata.id":"/redfish/v1/SessionService/Sessions"}},
                    "Systems":{"@odata.id":"/redfish/v1/Systems"}, "Chassis":{"@odata.id":"/redfish/v1/Chassis"}
                }),
            ),
        ),
        (
            "/redfish/v1/Systems".into(),
            resource(
                "/redfish/v1/Systems",
                "#ComputerSystemCollection.ComputerSystemCollection",
                "Systems",
                json!({"Members":[{"@odata.id":system}]}),
            ),
        ),
        (
            system.into(),
            resource(
                system,
                "#ComputerSystem.v1_15_0.ComputerSystem",
                "Tray",
                json!({"Memory":{"@odata.id":format!("{system}/Memory")},"Storage":{"@odata.id":format!("{system}/Storage")}}),
            ),
        ),
        (
            format!("{system}/Memory"),
            resource(
                &format!("{system}/Memory"),
                "#MemoryCollection.MemoryCollection",
                "Memory",
                json!({"Members":[{"@odata.id":memory}]}),
            ),
        ),
        (
            memory.clone(),
            resource(
                &memory,
                "#Memory.v1_20_0.Memory",
                "DIMM0",
                json!({"MemoryDeviceType":"DDR5", "CapacityMiB":131072,"SerialNumber":"DIMM-SN", "PartNumber":"DIMM-PN", "Manufacturer":"Example", "DeviceLocator":"Slot 0", "Location":{"PartLocation":{"ServiceLabel":"DIMM 0"}}, "Status":{"Health":"Warning","State":"Enabled"}}),
            ),
        ),
        (
            format!("{system}/Storage"),
            resource(
                &format!("{system}/Storage"),
                "#StorageCollection.StorageCollection",
                "Storage",
                json!({"Members":[{"@odata.id":storage}]}),
            ),
        ),
        (
            storage.clone(),
            resource(
                &storage,
                "#Storage.v1_15_0.Storage",
                "NVMe",
                json!({"Drives":[{"@odata.id":drive}]}),
            ),
        ),
        (
            drive.clone(),
            resource(
                &drive,
                "#Drive.v1_17_0.Drive",
                "SSD0",
                json!({"Model":"SSD","SerialNumber":"SSD-SN","CapacityBytes":3840000000000i64,"Protocol":"NVMe","MediaType":"SSD","Revision":"1.2", "Status":{"Health":"OK"}}),
            ),
        ),
        (
            "/redfish/v1/Chassis".into(),
            resource(
                "/redfish/v1/Chassis",
                "#ChassisCollection.ChassisCollection",
                "Chassis",
                json!({"Members":[{"@odata.id":chassis}]}),
            ),
        ),
        (
            chassis.into(),
            resource(
                chassis,
                "#Chassis.v1_25_0.Chassis",
                "Tray",
                json!({"ChassisType":"Module", "PowerSubsystem":{"@odata.id":ps},"Status":{"Health":"OK"},"Location":{"PartLocation":{"ServiceLabel":"Tray 0"}}}),
            ),
        ),
        (
            ps.clone(),
            resource(
                &ps,
                "#PowerSubsystem.v1_1_0.PowerSubsystem",
                "PowerSubsystem",
                json!({"PowerSupplies":{"@odata.id":format!("{ps}/PowerSupplies")}}),
            ),
        ),
        (
            format!("{ps}/PowerSupplies"),
            resource(
                &format!("{ps}/PowerSupplies"),
                "#PowerSupplyCollection.PowerSupplyCollection",
                "PowerSupplies",
                json!({"Members":[{"@odata.id":psu}]}),
            ),
        ),
        (
            psu.clone(),
            resource(
                &psu,
                "#PowerSupply.v1_5_0.PowerSupply",
                "PSU0",
                json!({"Model":"PSU","SerialNumber":"PSU-SN","PartNumber":"PSU-PN","FirmwareVersion":"2.1","PowerCapacityWatts":3000.0,"Status":{"Health":"Critical"}}),
            ),
        ),
    ];
    if legacy_power {
        routes.retain(|(path, _)| !path.starts_with(&ps));
        let power_path = format!("{chassis}/Power");
        for (path, value) in &mut routes {
            if path == chassis {
                value.as_object_mut().unwrap().remove("PowerSubsystem");
                value["Power"] = json!({"@odata.id":power_path});
            }
        }
        routes.push((power_path.clone(),resource(&power_path,"#Power.v1_7_0.Power","Power", json!({
            "PowerSupplies":[{"@odata.id":format!("{power_path}#/PowerSupplies/0"),"MemberId":"PSU0",
            "Model":"Legacy PSU","SerialNumber":"PSU-SN","PartNumber":"PSU-PN","FirmwareVersion":"2.1",
            "PowerCapacityWatts":3000,"Status":{"Health":"Warning"}}]
        }))));
    }
    if broken {
        for (path, value) in &mut routes {
            if path == &memory || path == &drive || path == &psu {
                value["Id"] = json!(42);
            }
        }
    }
    for (path, value) in routes {
        bmc.expect(Expect::get(&path, value.to_string()));
    }
    let root = ServiceRoot::new(bmc.clone()).await.unwrap();
    collect_report_from_root(&root, bmc.as_ref()).await.unwrap()
}

#[tokio::test]
async fn component_inventory_retains_parent_identity_fields_and_status() {
    let report = collect(false, false).await;
    assert!(report.errors.is_empty(), "{:?}", report.errors);
    let system = serde_json::to_value(&report.systems[0]).unwrap();
    let memory = &system["memory"][0];
    assert_eq!(memory["systemId"], "Tray");
    assert_eq!(memory["capacityMib"], 131072);
    assert_eq!(memory["memoryDeviceType"], "DDR5");
    assert_eq!(memory["serialNumber"], "DIMM-SN");
    assert_eq!(memory["partNumber"], "DIMM-PN");
    assert_eq!(memory["manufacturer"], "Example");
    assert_eq!(memory["deviceLocator"], "Slot 0");
    assert_eq!(memory["location"]["partLocation"]["serviceLabel"], "DIMM 0");
    assert_eq!(memory["status"]["health"], "Warning");
    let storage = &system["storage"][0];
    assert_eq!(storage["id"], "NVMe");
    assert_eq!(storage["systemId"], "Tray");
    let drive = &storage["drives"][0];
    assert_eq!(drive["id"], "SSD0");
    assert_eq!(drive["storageId"], "NVMe");
    assert_eq!(drive["systemId"], "Tray");
    assert_eq!(drive["model"], "SSD");
    assert_eq!(drive["serialNumber"], "SSD-SN");
    assert_eq!(drive["capacityBytes"], 3840000000000i64);
    assert_eq!(drive["protocol"], "NVMe");
    assert_eq!(drive["mediaType"], "SSD");
    assert_eq!(drive["revision"], "1.2");
    assert_eq!(drive["status"]["health"], "OK");
    let chassis = serde_json::to_value(&report.chassis[0]).unwrap();
    let psu = &chassis["powerSupplies"][0];
    assert_eq!(psu["id"], "PSU0");
    assert_eq!(psu["chassisId"], "Tray");
    assert_eq!(psu["model"], "PSU");
    assert_eq!(psu["serialNumber"], "PSU-SN");
    assert_eq!(psu["partNumber"], "PSU-PN");
    assert_eq!(psu["firmwareVersion"], "2.1");
    assert_eq!(psu["powerCapacityWatts"], 3000.0);
    assert_eq!(psu["status"]["health"], "Critical");
}

#[tokio::test]
async fn component_failures_are_scoped_to_parent_and_keep_other_domains() {
    let report = collect(true, false).await;
    assert_eq!(report.systems.len(), 1);
    assert_eq!(report.chassis.len(), 1);
    assert!(report.systems[0].memory.is_empty());
    assert_eq!(report.systems[0].storage[0].id, "NVMe");
    assert!(report.systems[0].storage[0].drives.is_empty());
    assert!(report.chassis[0].power_supplies.is_empty());
    let mut domains = report
        .errors
        .iter()
        .map(|error| {
            assert_eq!(error.resource.as_deref(), Some("Tray"));
            error.domain.as_str()
        })
        .collect::<Vec<_>>();
    domains.sort();
    assert_eq!(
        domains,
        ["chassis.powerSupplies", "systems.memory", "systems.storage"]
    );
}

#[tokio::test]
async fn legacy_power_supplies_preserve_member_identity_and_status() {
    let report = collect(false, true).await;
    assert!(report.errors.is_empty(), "{:?}", report.errors);
    let psu = &report.chassis[0].power_supplies[0];
    assert_eq!(psu.id, "PSU0");
    assert_eq!(psu.chassis_id, "Tray");
    assert_eq!(psu.model.as_deref(), Some("Legacy PSU"));
    assert_eq!(psu.serial_number.as_deref(), Some("PSU-SN"));
    assert_eq!(psu.part_number.as_deref(), Some("PSU-PN"));
    assert_eq!(psu.firmware_version.as_deref(), Some("2.1"));
    assert_eq!(psu.power_capacity_watts, Some(3000.0));
    assert_eq!(
        psu.status.as_ref().unwrap().health.as_deref(),
        Some("Warning")
    );
    assert_eq!(
        psu.odata_id.as_ref().unwrap().as_ref(),
        "/redfish/v1/Chassis/Tray/Power#/PowerSupplies/0"
    );
}
