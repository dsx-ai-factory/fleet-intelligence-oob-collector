// Copyright (c) 2026, NVIDIA CORPORATION & AFFILIATES. All rights reserved.
// SPDX-License-Identifier: Apache-2.0

use std::collections::BTreeMap;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use bmc_dispatch::{Dispatcher, DispatcherConfig, DispatchingHttpClient};
use bmc_inventory::config::Target;
use bmc_inventory::models::{NodeInventory, TargetKind};
use bmc_inventory::source::{InventorySource, LatencyProbe, RedfishSource};
use nv_redfish::bmc_http::{BmcCredentials, HttpClient};
use nv_redfish::core::{BmcError, BmcErrorClass, NoRepair};
use serde_json::{Value, json};
use tokio::io::{AsyncReadExt, AsyncWriteExt};

const MAC: &str = "00:11:22:33:44:55";

/// Synthetic Redfish responses served through the collector's real HTTP path.
struct Fixture {
    target: Target,
    requests: Arc<Mutex<Vec<String>>>,
    server: tokio::task::JoinHandle<()>,
}

impl Drop for Fixture {
    fn drop(&mut self) {
        self.server.abort();
    }
}

impl Fixture {
    async fn new(root: Value, chassis: Value, extra: BTreeMap<String, Value>) -> Self {
        let mut routes = extra;
        routes.insert("/redfish/v1".into(), root);
        routes.insert("/redfish/v1/Chassis/1".into(), chassis);
        // Lite-On's root repair also supplies an unadvertised UpdateService link.
        routes.insert(
            "/redfish/v1/UpdateService".into(),
            json!({
                "@odata.id": "/redfish/v1/UpdateService",
                "@odata.type": "#UpdateService.v1_11_0.UpdateService",
                "Id": "UpdateService", "Name": "Update service"
            }),
        );
        for (path, schema, members) in [
            ("Managers", "ManagerCollection", json!([])),
            ("Systems", "ComputerSystemCollection", json!([])),
            (
                "Chassis",
                "ChassisCollection",
                json!([{"@odata.id": "/redfish/v1/Chassis/1"}]),
            ),
        ] {
            routes.insert(
                format!("/redfish/v1/{path}"),
                json!({
                    "@odata.id": format!("/redfish/v1/{path}"),
                    "@odata.type": format!("#{schema}.{schema}"),
                    "Name": path, "Members": members
                }),
            );
        }
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let port = listener.local_addr().unwrap().port();
        let requests = Arc::new(Mutex::new(Vec::new()));
        let recorded = Arc::clone(&requests);
        let server = tokio::spawn(async move {
            while let Ok((mut stream, _)) = listener.accept().await {
                let mut request = Vec::new();
                let mut buffer = [0; 1024];
                while !request.windows(4).any(|window| window == b"\r\n\r\n") {
                    let read = stream.read(&mut buffer).await.unwrap();
                    if read == 0 {
                        break;
                    }
                    request.extend_from_slice(&buffer[..read]);
                }
                let path = String::from_utf8_lossy(&request)
                    .split_whitespace()
                    .nth(1)
                    .unwrap()
                    .to_string();
                recorded.lock().unwrap().push(path.clone());
                // Query strings are deliberately unmatched: affected BMCs reject $expand.
                let (status, body) = match routes.get(path.trim_end_matches('/')) {
                    Some(body) => ("200 OK", body.to_string()),
                    None => (
                        "404 Not Found",
                        json!({"error": "fixture route unavailable"}).to_string(),
                    ),
                };
                let response = format!(
                    "HTTP/1.1 {status}\r\ncontent-type: application/json\r\ncontent-length: {}\r\nconnection: close\r\n\r\n{body}",
                    body.len()
                );
                stream.write_all(response.as_bytes()).await.unwrap();
            }
        });
        Self {
            target: Target {
                kind: TargetKind::Compute,
                name: None,
                compute_zone: None,
                node_group: None,
                ip: "127.0.0.1".parse().unwrap(),
                port: Some(port),
                mac: MAC.parse().unwrap(),
                address: format!("http://127.0.0.1:{port}"),
                credentials_profile: None,
                username: "fixture".into(),
                password: "fixture".into(),
                labels: Default::default(),
            },
            requests,
            server,
        }
    }

    fn dispatcher(&self) -> Arc<Dispatcher> {
        Arc::new(
            Dispatcher::start(
                DispatcherConfig {
                    global_max_in_flight: 1,
                    bmc_max_in_flight: 1,
                    bmc_requests_per_second: 100,
                    bmc_burst: 100,
                    workflow_queue_capacity: 8,
                },
                [MAC.to_string()],
            )
            .unwrap(),
        )
    }

    async fn collect(&self) -> NodeInventory {
        RedfishSource::new(false, Duration::from_secs(5), self.dispatcher())
            .unwrap()
            .with_latency_probe(LatencyProbe::Disabled)
            .collect(self.target.clone())
            .await
    }
}

fn root(vendor: &str) -> Value {
    json!({
        "@odata.id": "/redfish/v1", "@odata.type": "#ServiceRoot.v1_15_0.ServiceRoot",
        "Id": "RootService", "Name": "Root Service", "RedfishVersion": "1.15.1",
        "Vendor": vendor,
        "Links": {"Sessions": {"@odata.id": "/redfish/v1/SessionService/Sessions"}},
        "Chassis": {"@odata.id": "/redfish/v1/Chassis"}
    })
}

fn chassis() -> Value {
    json!({
        "@odata.id": "/redfish/v1/Chassis/1", "@odata.type": "#Chassis.v1_25_0.Chassis",
        "Id": "1", "Name": "Fixture chassis", "ChassisType": "RackMount",
        "Model": "Fixture model", "SerialNumber": "SYNTHETIC-1"
    })
}

#[tokio::test]
async fn invalid_chassis_uuids_preserve_inventory_on_affected_platforms() {
    for (vendor, product, uuid) in [
        ("Dell", "Fixture", "$DEVICE_UUID"),
        ("WIWYNN", "Fixture", "STATIC:3:0"),
        ("Nvidia", "BlueField-3 DPU", ""),
    ] {
        let mut root = root(vendor);
        root["Product"] = json!(product);
        let mut chassis = chassis();
        chassis["UUID"] = json!(uuid);
        let fixture = Fixture::new(root, chassis, BTreeMap::new()).await;
        let inventory = fixture.collect().await;
        assert!(
            inventory.target_error.is_none(),
            "{vendor}: {:?}",
            inventory.target_error
        );
        assert!(
            inventory.domain_errors.is_empty(),
            "{vendor}: {:?}",
            inventory.domain_errors
        );
        assert_eq!(inventory.chassis.len(), 1, "{vendor}");
        assert_eq!(
            inventory.chassis[0].serial_number.as_deref(),
            Some("SYNTHETIC-1")
        );
        assert_eq!(inventory.source.mac, MAC);
    }
}

#[tokio::test]
async fn gb300_inventory_avoids_advertised_but_broken_expand() {
    for mut root in [root("AMI"), root("NVIDIA")] {
        root["Product"] = json!("GB BMC");
        root["Oem"] = json!({"Ami": {"RtpVersion": "13.09.1"}});
        root["ProtocolFeaturesSupported"] = json!({
            "ExpandQuery": {"ExpandAll": true, "NoLinks": true}
        });
        let fixture = Fixture::new(root, chassis(), BTreeMap::new()).await;
        let inventory = fixture.collect().await;
        assert!(inventory.target_error.is_none());
        assert!(
            inventory.domain_errors.is_empty(),
            "{:?}",
            inventory.domain_errors
        );
        assert_eq!(inventory.chassis.len(), 1);
        let requests = fixture.requests.lock().unwrap();
        assert!(requests.iter().any(|path| path == "/redfish/v1/Chassis/1"));
        assert!(
            requests.iter().all(|path| !path.contains('?')),
            "{requests:?}"
        );
    }
}

#[tokio::test]
async fn liteon_power_supply_repairs_preserve_state_and_inventory() {
    for (state, expected) in [("Standby", "StandbySpare"), ("Sleep", "StandbyOffline")] {
        let mut chassis = chassis();
        chassis["Status"] = json!({"State": state});
        chassis["PowerSubsystem"] = json!({"@odata.id": "/redfish/v1/Chassis/1/PowerSubsystem"});
        let fixture = Fixture::new(root("LITE-ON"), chassis, BTreeMap::from([
            ("/redfish/v1/Chassis/1/PowerSubsystem".into(), json!({
                "@odata.id": "/redfish/v1/Chassis/1/PowerSubsystem",
                "@odata.type": "#PowerSubsystem.v1_1_0.PowerSubsystem", "Id": "PowerSubsystem", "Name": "Power",
                "PowerSupplies": {"@odata.id": "/redfish/v1/Chassis/1/PowerSubsystem/PowerSupplies"}
            })),
            ("/redfish/v1/Chassis/1/PowerSubsystem/PowerSupplies".into(), json!({
                "@odata.id": "/redfish/v1/Chassis/1/PowerSubsystem/PowerSupplies",
                "@odata.type": "#PowerSupplyCollection.PowerSupplyCollection", "Name": "Power supplies",
                "Members": [{"@odata.id": "/redfish/v1/Chassis/1/PowerSubsystem/PowerSupplies/1"}]
            })),
            ("/redfish/v1/Chassis/1/PowerSubsystem/PowerSupplies/1".into(), json!({
                "@odata.id": "/redfish/v1/Chassis/1/PowerSubsystem/PowerSupplies/1",
                "@odata.type": "#PowerSupply.v1_5_0.PowerSupply", "Id": "1", "Name": "PSU",
                "Model": "Fixture PSU", "Status": {"State": state},
                "LocationIndicatorActive": {"State": "Off"}
            })),
        ])).await;
        let inventory = fixture.collect().await;
        assert!(inventory.target_error.is_none());
        assert!(
            inventory.domain_errors.is_empty(),
            "{:?}",
            inventory.domain_errors
        );
        let chassis = &inventory.chassis[0];
        assert_eq!(
            chassis.status.as_ref().unwrap().state.as_deref(),
            Some(expected)
        );
        assert_eq!(chassis.power_supplies.len(), 1);
        assert_eq!(
            chassis.power_supplies[0].model.as_deref(),
            Some("Fixture PSU")
        );
        assert_eq!(
            chassis.power_supplies[0]
                .status
                .as_ref()
                .unwrap()
                .state
                .as_deref(),
            Some(expected)
        );
    }
}

#[tokio::test]
async fn dispatched_errors_preserve_decode_failures_and_response_bodies() {
    let fixture = Fixture::new(root("Fixture"), chassis(), BTreeMap::new()).await;
    let dispatcher = fixture.dispatcher();
    let client = DispatchingHttpClient::new(
        nv_redfish::bmc_http::reqwest::Client::new().unwrap(),
        dispatcher.inventory_gate(MAC).unwrap(),
    );
    let credentials = BmcCredentials::new("fixture".into(), "fixture".into());
    let url = format!("{}/redfish/v1", fixture.target.address)
        .parse()
        .unwrap();
    let error = client
        .get::<u32>(url, &credentials, None, &Default::default(), &NoRepair)
        .await
        .unwrap_err();
    assert_eq!(error.error_class(), BmcErrorClass::ResponseParse);
    assert!(error.into_decode_error().is_ok());

    let url = format!("{}/unavailable", fixture.target.address)
        .parse()
        .unwrap();
    let error = client
        .get::<Value>(url, &credentials, None, &Default::default(), &NoRepair)
        .await
        .unwrap_err();
    assert_eq!(
        error.error_class(),
        BmcErrorClass::HttpResponse { status: 404 }
    );
    assert!(
        error
            .response_body()
            .unwrap()
            .contains("fixture route unavailable")
    );
    assert!(error.into_decode_error().is_err());
}
