// Copyright (c) 2026, NVIDIA CORPORATION & AFFILIATES. All rights reserved.
// SPDX-License-Identifier: Apache-2.0
#![cfg(unix)]

use std::path::PathBuf;
use std::process::Stdio;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::Duration;

use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::{TcpListener, TcpStream};
use tokio::process::{Child, Command};
use tokio::time::timeout;

static NEXT_CONFIG: AtomicU64 = AtomicU64::new(0);

struct Runtime {
    child: Child,
    config: PathBuf,
    credentials: PathBuf,
}

impl Runtime {
    fn start(backend_port: u16, bmc_port: u16) -> Self {
        Self::start_with_interval(backend_port, bmc_port, 86400)
    }

    fn start_with_interval(backend_port: u16, bmc_port: u16, interval_seconds: u64) -> Self {
        let config = std::env::temp_dir().join(format!(
            "fleetint-shutdown-{}-{}.toml",
            std::process::id(),
            NEXT_CONFIG.fetch_add(1, Ordering::Relaxed)
        ));
        let credentials = config.with_extension("json");
        std::fs::write(
            &credentials,
            r#"{"profiles":{"bmc":{"username":"example-user","password":"example-password"}}}"#,
        )
        .expect("write test credentials");
        std::fs::write(
            &config,
            format!(
                "[credentials]\nfile = {:?}\n[[targets]]\nip = \"127.0.0.1\"\nport = {bmc_port}\nmac = \"02:00:00:00:00:01\"\ncredentials_profile = \"bmc\"\n",
                credentials.to_str().expect("UTF-8 credential path")
            ),
        )
        .expect("write test configuration");
        let child = Command::new(env!("CARGO_BIN_EXE_bmc-collector"))
            .args(["run", "--config"])
            .arg(&config)
            .args([
                "--disable-latency-probe",
                "--target-concurrency",
                "1",
                "--timeout-seconds",
                "30",
            ])
            .args(["--interval-seconds", &interval_seconds.to_string()])
            .env(
                "FLEETINT_BACKEND_URL",
                format!("http://127.0.0.1:{backend_port}"),
            )
            .env("FLEETINT_AGENT_TOKEN", "example-token")
            .env("NO_PROXY", "*")
            .env("RUST_LOG", "info")
            .stdout(Stdio::null())
            .stderr(Stdio::piped())
            .kill_on_drop(true)
            .spawn()
            .expect("start collector");
        Self {
            child,
            config,
            credentials,
        }
    }

    async fn signal(&self, signal: &str) {
        let result = Command::new("kill")
            .args([
                signal,
                &self.child.id().expect("running collector").to_string(),
            ])
            .status()
            .await
            .expect("send signal to test child");
        assert!(result.success());
    }

    async fn expect_clean_exit(&mut self) {
        let status = timeout(Duration::from_secs(3), self.child.wait())
            .await
            .expect("collector should stop promptly")
            .expect("wait for collector");
        assert!(
            status.success(),
            "collector did not shut down cleanly: {status}"
        );
    }
}

impl Drop for Runtime {
    fn drop(&mut self) {
        let _ = std::fs::remove_file(&self.config);
        let _ = std::fs::remove_file(&self.credentials);
    }
}

async fn accept_request(listener: &TcpListener) -> (TcpStream, String) {
    timeout(Duration::from_secs(5), async {
        let (mut stream, _) = listener.accept().await.expect("accept request");
        let mut bytes = Vec::new();
        loop {
            let mut buffer = [0; 4096];
            let length = stream.read(&mut buffer).await.expect("read request");
            assert!(length > 0, "request ended before its body");
            bytes.extend_from_slice(&buffer[..length]);
            if let Some(end) = bytes.windows(4).position(|window| window == b"\r\n\r\n") {
                let headers = String::from_utf8_lossy(&bytes[..end]);
                let body_length = headers
                    .lines()
                    .filter_map(|line| line.split_once(':'))
                    .find(|(key, _)| key.eq_ignore_ascii_case("content-length"))
                    .map_or(0, |(_, value)| {
                        value.trim().parse::<usize>().expect("length")
                    });
                if bytes.len() >= end + 4 + body_length {
                    return (stream, String::from_utf8(bytes).expect("UTF-8 request"));
                }
            }
        }
    })
    .await
    .expect("collector request timeout")
}

async fn reply(stream: &mut TcpStream, body: &str) {
    reply_status(stream, 200, body).await;
}

async fn reply_status(stream: &mut TcpStream, status: u16, body: &str) {
    stream
        .write_all(
            format!(
                "HTTP/1.1 {status} Test\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
                body.len()
            )
            .as_bytes(),
        )
        .await
        .expect("reply to collector");
    stream.shutdown().await.expect("close response");
}

#[tokio::test]
async fn signals_stop_during_pending_enrollment() {
    for signal in ["-INT", "-TERM"] {
        let backend = TcpListener::bind("127.0.0.1:0").await.expect("backend");
        let mut runtime = Runtime::start(backend.local_addr().unwrap().port(), 1);
        let (_request, headers) = accept_request(&backend).await;
        assert!(headers.starts_with("POST /v1/agent/enroll "));
        runtime.signal(signal).await;
        runtime.expect_clean_exit().await;
    }
}

#[tokio::test]
async fn sigint_stops_during_pending_redfish_collection() {
    let backend = TcpListener::bind("127.0.0.1:0").await.expect("backend");
    let bmc = TcpListener::bind("127.0.0.1:0").await.expect("mock BMC");
    let mut runtime = Runtime::start(
        backend.local_addr().unwrap().port(),
        bmc.local_addr().unwrap().port(),
    );
    let (mut enrollment, _) = accept_request(&backend).await;
    reply(&mut enrollment, r#"{"jwtAssertion":"example-jwt"}"#).await;
    let (_pending_redfish, _) = timeout(Duration::from_secs(5), bmc.accept())
        .await
        .expect("collector should start Redfish collection")
        .expect("accept mock Redfish connection");
    runtime.signal("-INT").await;
    runtime.expect_clean_exit().await;
}

#[tokio::test]
async fn sigterm_finishes_in_flight_inventory_upload_before_exiting() {
    let backend = TcpListener::bind("127.0.0.1:0").await.expect("backend");
    let unavailable = TcpListener::bind("127.0.0.1:0").await.expect("unused port");
    let bmc_port = unavailable.local_addr().unwrap().port();
    drop(unavailable);
    let mut runtime = Runtime::start(backend.local_addr().unwrap().port(), bmc_port);
    let (mut enrollment, _) = accept_request(&backend).await;
    reply(&mut enrollment, r#"{"jwtAssertion":"example-jwt"}"#).await;
    let (mut upload, request) = accept_request(&backend).await;
    assert!(request.starts_with("PUT /v1/agent/oob_nodes "));
    runtime.signal("-TERM").await;
    assert!(
        timeout(Duration::from_millis(100), runtime.child.wait())
            .await
            .is_err()
    );
    reply(&mut upload, r#"{"results":[{"status":"OK"}]}"#).await;
    runtime.expect_clean_exit().await;
}

async fn finish_attestation(backend: &TcpListener, runtime: &mut Runtime) {
    complete_attestation(backend, "02:00:00:00:00:01", "OK").await;
    runtime.signal("-TERM").await;
    runtime.expect_clean_exit().await;
}

async fn complete_attestation(backend: &TcpListener, mac: &str, upload_status: &str) {
    let (mut nonce, request) = accept_request(backend).await;
    assert!(request.starts_with(&format!("POST /v1/agent/oob_nodes/{mac}/nonce ")));
    reply(
        &mut nonce,
        r#"{"nonce":"example-nonce","jwtAssertion":"rotated-jwt","nonceRefreshTimestamp":"2026-01-01T00:00:00Z"}"#,
    )
    .await;
    let (mut upload, request) = accept_request(backend).await;
    assert!(request.starts_with("POST /v1/agent/oob_nodes/attestation "));
    assert!(request.contains(mac));
    assert!(
        request
            .to_ascii_lowercase()
            .contains("authorization: bearer rotated-jwt")
    );
    reply(
        &mut upload,
        &format!(r#"{{"results":[{{"bmcMac":"{mac}","status":"{upload_status}"}}]}}"#),
    )
    .await;
}

#[tokio::test]
async fn cycle_uses_one_target_and_credential_snapshot_without_inventory_uuid() {
    let backend = TcpListener::bind("127.0.0.1:0").await.expect("backend");
    let unavailable = TcpListener::bind("127.0.0.1:0").await.expect("unused port");
    let bmc_port = unavailable.local_addr().unwrap().port();
    drop(unavailable);
    let mut runtime = Runtime::start(backend.local_addr().unwrap().port(), bmc_port);
    let (mut enrollment, _) = accept_request(&backend).await;
    reply(&mut enrollment, r#"{"jwtAssertion":"example-jwt"}"#).await;
    let (mut inventory, request) = accept_request(&backend).await;
    assert!(request.starts_with("PUT /v1/agent/oob_nodes "));
    // Rotate targets while the inventory upload is pending, then remove the
    // credential file. Attestation must use the snapshot already registered
    // with the dispatcher, rather than reload either file mid-cycle.
    let config = std::fs::read_to_string(&runtime.config).unwrap();
    std::fs::write(
        &runtime.config,
        config.replace("02:00:00:00:00:01", "02:00:00:00:00:02"),
    )
    .unwrap();
    std::fs::remove_file(&runtime.credentials).unwrap();
    reply(&mut inventory, r#"{"results":[{"status":"OK"}]}"#).await;
    finish_attestation(&backend, &mut runtime).await;
}

#[tokio::test]
async fn inventory_reenrollment_failure_does_not_skip_attestation_for_existing_sources() {
    let backend = TcpListener::bind("127.0.0.1:0").await.expect("backend");
    let unavailable = TcpListener::bind("127.0.0.1:0").await.expect("unused port");
    let bmc_port = unavailable.local_addr().unwrap().port();
    drop(unavailable);
    let mut runtime = Runtime::start(backend.local_addr().unwrap().port(), bmc_port);
    let (mut enrollment, _) = accept_request(&backend).await;
    reply(&mut enrollment, r#"{"jwtAssertion":"example-jwt"}"#).await;
    let (mut inventory, request) = accept_request(&backend).await;
    assert!(request.starts_with("PUT /v1/agent/oob_nodes "));
    reply_status(&mut inventory, 401, r#"{"error":"expired"}"#).await;
    let (mut reenrollment, request) = accept_request(&backend).await;
    assert!(request.starts_with("POST /v1/agent/enroll "));
    reply_status(&mut reenrollment, 503, r#"{"error":"unavailable"}"#).await;
    // The nonce endpoint may still serve an enrolled source with the retained
    // token. An inventory-phase error must not suppress the attempt.
    finish_attestation(&backend, &mut runtime).await;
}

#[tokio::test]
async fn next_cycle_reloads_targets_and_dispatcher_membership() {
    let backend = TcpListener::bind("127.0.0.1:0").await.expect("backend");
    let unavailable = TcpListener::bind("127.0.0.1:0").await.expect("unused port");
    let bmc_port = unavailable.local_addr().unwrap().port();
    drop(unavailable);
    let mut runtime =
        Runtime::start_with_interval(backend.local_addr().unwrap().port(), bmc_port, 1);
    let (mut enrollment, _) = accept_request(&backend).await;
    reply(&mut enrollment, r#"{"jwtAssertion":"example-jwt"}"#).await;
    let (mut inventory, _) = accept_request(&backend).await;
    let config = std::fs::read_to_string(&runtime.config).unwrap();
    std::fs::write(
        &runtime.config,
        config.replace("02:00:00:00:00:01", "02:00:00:00:00:02"),
    )
    .unwrap();
    reply(&mut inventory, r#"{"results":[{"status":"OK"}]}"#).await;
    complete_attestation(&backend, "02:00:00:00:00:01", "OK").await;

    let (mut inventory, request) = accept_request(&backend).await;
    assert!(request.starts_with("PUT /v1/agent/oob_nodes "));
    assert!(request.contains("02:00:00:00:00:02"));
    assert!(!request.contains("02:00:00:00:00:01"));
    reply(&mut inventory, r#"{"results":[{"status":"OK"}]}"#).await;
    complete_attestation(&backend, "02:00:00:00:00:02", "OK").await;
    runtime.signal("-TERM").await;
    runtime.expect_clean_exit().await;
}

#[tokio::test]
async fn rejected_attestation_item_is_not_logged_as_successful_sync() {
    let backend = TcpListener::bind("127.0.0.1:0").await.expect("backend");
    let unavailable = TcpListener::bind("127.0.0.1:0").await.expect("unused port");
    let bmc_port = unavailable.local_addr().unwrap().port();
    drop(unavailable);
    let mut runtime = Runtime::start(backend.local_addr().unwrap().port(), bmc_port);
    let (mut enrollment, _) = accept_request(&backend).await;
    reply(&mut enrollment, r#"{"jwtAssertion":"example-jwt"}"#).await;
    let (mut inventory, _) = accept_request(&backend).await;
    reply(&mut inventory, r#"{"results":[{"status":"OK"}]}"#).await;
    complete_attestation(&backend, "02:00:00:00:00:01", "INTERNAL_ERROR").await;
    runtime.signal("-TERM").await;
    runtime.expect_clean_exit().await;
    let mut logs = String::new();
    runtime
        .child
        .stderr
        .take()
        .unwrap()
        .read_to_string(&mut logs)
        .await
        .unwrap();
    assert!(
        logs.contains("attestation mini-batch export failed"),
        "{logs}"
    );
    assert!(
        !logs.contains("attestation backend sync completed"),
        "{logs}"
    );
}
