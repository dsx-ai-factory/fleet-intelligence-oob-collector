// Copyright (c) 2026, NVIDIA CORPORATION & AFFILIATES. All rights reserved.
// SPDX-License-Identifier: Apache-2.0

use std::io::{BufRead, BufReader};
use std::path::PathBuf;
use std::process::{Child, Command, Stdio};
use std::sync::Arc;
use std::time::Duration;

use bmc_attestation::source::{MeasurementRequest, NonceData, RedfishAttestationSource};
use bmc_dispatch::{Dispatcher, DispatcherConfig, proxy::NicoBmcProxyConfig};
use bmc_inventory::source::{InventorySource, LatencyProbe, RedfishSource};

struct ProxyFixture {
    child: Child,
    directory: PathBuf,
    config: NicoBmcProxyConfig,
}

impl ProxyFixture {
    fn start() -> Self {
        let directory = std::env::temp_dir().join(format!("fi-proxy-tls-{}", std::process::id()));
        std::fs::create_dir_all(&directory).unwrap();
        let mut child = Command::new("python3")
            .arg(concat!(
                env!("CARGO_MANIFEST_DIR"),
                "/tests/fixtures/nico-proxy/server.py"
            ))
            .arg(&directory)
            .stdout(Stdio::piped())
            .spawn()
            .unwrap();
        let mut port = String::new();
        BufReader::new(child.stdout.take().unwrap())
            .read_line(&mut port)
            .unwrap();
        let config = NicoBmcProxyConfig {
            url: format!(
                "https://127.0.0.1:{}",
                port.trim().parse::<u16>().expect("fixture port")
            ),
            ca_file: directory.join("ca.crt"),
            client_cert_file: directory.join("client.crt"),
            client_key_file: directory.join("client.key"),
        };
        Self {
            child,
            directory,
            config,
        }
    }
}

impl Drop for ProxyFixture {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
        let _ = std::fs::remove_dir_all(&self.directory);
    }
}

#[tokio::test]
async fn inventory_and_attestation_use_mtls_proxy_without_device_credentials() {
    let fixture = ProxyFixture::start();
    let mac = "00:11:22:33:44:55";
    let dispatcher = Arc::new(
        Dispatcher::start(
            DispatcherConfig {
                global_max_in_flight: 4,
                bmc_max_in_flight: 2,
                bmc_requests_per_second: 100,
                bmc_burst: 100,
                workflow_queue_capacity: 8,
            },
            [mac.to_string()],
        )
        .unwrap(),
    );
    let source = RedfishSource::new(true, Duration::from_secs(5), dispatcher.clone())
        .unwrap()
        .with_latency_probe(LatencyProbe::Enabled(Default::default()));
    let target = bmc_inventory::config::Target {
        proxy: Some(fixture.config.clone()),
        kind: bmc_inventory::models::TargetKind::Compute,
        name: None,
        compute_zone: None,
        node_group: None,
        ip: "192.0.2.1".parse().unwrap(),
        port: None,
        mac: mac.parse().unwrap(),
        address: "192.0.2.1".into(),
        credentials_profile: None,
        // Even accidental device credentials must never leave through the proxy.
        username: "must-not-send".into(),
        password: "must-not-send".into(),
        labels: Default::default(),
    };
    let inventory = source.collect(target.clone()).await;
    assert!(
        inventory.target_error.is_none(),
        "{:?}",
        inventory.target_error
    );
    assert_eq!(inventory.source.redfish_version.as_deref(), Some("1.15.1"));
    assert!(inventory.latency.is_empty());

    let attestation = RedfishAttestationSource::new(
        443,
        true,
        Duration::from_secs(5),
        Duration::from_secs(5),
        Duration::from_millis(1),
    )
    .unwrap();
    let record = attestation
        .collect(
            bmc_attestation::config::Target {
                proxy: Some(fixture.config.clone()),
                name: mac.into(),
                bmc_mac: mac.into(),
                address: "192.0.2.1".into(),
                credentials_profile: None,
                username: "must-not-send".into(),
                password: "must-not-send".into(),
            },
            NonceData {
                nonce: "backend-issued-test-nonce".into(),
                nonce_refresh_timestamp: None,
            },
            MeasurementRequest {
                measurement_indices: vec![255],
            },
            dispatcher.attestation_gate(mac).unwrap(),
            &|_| panic!("proxy must never reload device credentials"),
        )
        .await;
    let report = record.result.unwrap();
    assert!(report.errors.is_empty(), "{:?}", report.errors);
    assert_eq!(report.evidences.len(), 1);
    assert_eq!(report.evidences[0].nonce, "backend-issued-test-nonce");
    assert_eq!(report.evidences[0].evidence, "signed-test-evidence");
    let requests = std::fs::read_to_string(fixture.directory.join("requests")).unwrap();
    assert_eq!(
        requests
            .lines()
            .filter(|line| line.starts_with("POST "))
            .count(),
        1
    );
    assert!(requests.contains("GET /redfish/v1/TaskService/Tasks/1"));

    let no_identity = reqwest::Client::builder()
        .tls_built_in_root_certs(false)
        .add_root_certificate(
            reqwest::Certificate::from_pem(&std::fs::read(&fixture.config.ca_file).unwrap())
                .unwrap(),
        )
        .build()
        .unwrap();
    assert!(
        no_identity
            .get(format!("{}/redfish/v1", fixture.config.url))
            .send()
            .await
            .is_err(),
        "proxy requires a client certificate"
    );

    let mut untrusted = target.clone();
    untrusted.proxy.as_mut().unwrap().ca_file = fixture.config.client_cert_file.clone();
    assert!(
        source.collect(untrusted).await.target_error.is_some(),
        "--insecure-tls must not disable proxy certificate verification"
    );

    // A rotated invalid identity must fail closed even with --insecure-tls.
    std::fs::write(&fixture.config.client_key_file, "invalid rotated key").unwrap();
    let failed = source.collect(target).await;
    assert!(failed.target_error.is_some());
    assert_eq!(
        requests,
        std::fs::read_to_string(fixture.directory.join("requests")).unwrap()
    );
}
