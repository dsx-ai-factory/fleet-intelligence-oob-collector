// Copyright (c) 2026, NVIDIA CORPORATION & AFFILIATES. All rights reserved.
// SPDX-License-Identifier: Apache-2.0
#![cfg(unix)]

use std::path::PathBuf;
use std::process::Stdio;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::Duration;

use tokio::io::{AsyncBufReadExt, AsyncReadExt, AsyncWriteExt, BufReader};
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
        Self::start_with_targets(backend_port, bmc_port, interval_seconds, 1, 1)
    }

    fn start_with_targets(
        backend_port: u16,
        bmc_port: u16,
        interval_seconds: u64,
        target_count: usize,
        concurrency: usize,
    ) -> Self {
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
        let targets = (1..=target_count).map(|index| format!(
            "[[targets]]\nip = \"127.0.0.1\"\nport = {bmc_port}\nmac = \"02:00:00:00:00:{index:02X}\"\ncredentials_profile = \"bmc\"\n"
        )).collect::<String>();
        std::fs::write(
            &config,
            format!(
                "[credentials]\nfile = {:?}\n{targets}",
                credentials.to_str().unwrap()
            ),
        )
        .expect("write test configuration");
        let child = Command::new(env!("CARGO_BIN_EXE_bmc-collector"))
            .args(["run", "--config"])
            .arg(&config)
            .args([
                "--disable-latency-probe",
                "--target-concurrency",
                &concurrency.to_string(),
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
        logs.contains("attestation target export rejected"),
        "{logs}"
    );
    assert!(
        !logs
            .lines()
            .any(|line| line.contains("attestation backend sync completed")
                && !line.contains("with upload failures")),
        "{logs}"
    );
}

#[tokio::test]
async fn partial_uploads_report_each_target_and_continue_later_batches() {
    for malformed in [false, true] {
        let backend = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let unavailable = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let bmc_port = unavailable.local_addr().unwrap().port();
        drop(unavailable);
        let mut runtime = Runtime::start_with_targets(
            backend.local_addr().unwrap().port(),
            bmc_port,
            86400,
            4,
            3,
        );
        let (mut enrollment, _) = accept_request(&backend).await;
        reply(&mut enrollment, r#"{"jwtAssertion":"example-jwt"}"#).await;
        let mut nonces = 0;
        while nonces < 4 {
            let (mut stream, request) = accept_request(&backend).await;
            if request.starts_with("PUT /v1/agent/oob_nodes ") {
                let body = request.split_once("\r\n\r\n").unwrap().1;
                // The fixture serializes one source object per inventory observation.
                let count = body.matches(r#""source":"#).count();
                assert!((1..=3).contains(&count));
                let results = vec![r#"{"status":"OK"}"#; count].join(",");
                reply(&mut stream, &format!(r#"{{"results":[{results}]}}"#)).await;
            } else {
                assert!(request.contains("/nonce "));
                nonces += 1;
                reply(&mut stream, r#"{"nonce":"example-nonce"}"#).await;
            }
        }
        let (mut upload, request) = accept_request(&backend).await;
        assert!(request.starts_with("POST /v1/agent/oob_nodes/attestation "));
        if malformed {
            reply(&mut upload, "not JSON").await;
        } else {
            reply(
                &mut upload,
                r#"{"results":[
                {"bmcMac":"02:00:00:00:00:02","status":"INTERNAL_ERROR"},
                {"bmcMac":"02:00:00:00:00:01","status":"OK"}
            ]}"#,
            )
            .await;
        }
        let (mut upload, request) = accept_request(&backend).await;
        assert!(request.contains("02:00:00:00:00:04"));
        assert!(!request.contains("02:00:00:00:00:01"));
        if !malformed {
            reply_status(&mut upload, 401, r#"{"error":"expired"}"#).await;
            let (mut reenrollment, request) = accept_request(&backend).await;
            assert!(request.starts_with("POST /v1/agent/enroll "));
            reply(&mut reenrollment, r#"{"jwtAssertion":"renewed-jwt"}"#).await;
            let (retry, request) = accept_request(&backend).await;
            assert!(request.contains("02:00:00:00:00:04"));
            assert!(!request.contains("02:00:00:00:00:01"));
            assert!(
                request
                    .to_ascii_lowercase()
                    .contains("authorization: bearer renewed-jwt")
            );
            upload = retry;
        }
        reply(
            &mut upload,
            r#"{"results":[{"bmcMac":"02:00:00:00:00:04","status":"OK"}]}"#,
        )
        .await;
        // Observe the completed summary before requesting shutdown.
        let mut stderr = BufReader::new(runtime.child.stderr.take().unwrap());
        let mut logs = String::new();
        timeout(Duration::from_secs(3), async {
            loop {
                let mut line = String::new();
                assert!(stderr.read_line(&mut line).await.unwrap() > 0);
                let completed = line.contains("sleeping until next cycle");
                logs.push_str(&line);
                if completed {
                    break;
                }
            }
        })
        .await
        .expect("upload summary");
        runtime.signal("-TERM").await;
        runtime.expect_clean_exit().await;
        assert!(
            logs.contains("attestation target export accepted"),
            "{logs}"
        );
        let summary = logs
            .lines()
            .find(|line| line.contains("attestation backend sync completed with upload failures"))
            .expect(&logs);
        if malformed {
            assert!(
                summary.contains("accepted_count=1") && summary.contains("unconfirmed_count=3"),
                "{summary}"
            );
        } else {
            assert!(
                summary.contains("accepted_count=2")
                    && summary.contains("rejected_count=1")
                    && summary.contains("unconfirmed_count=1"),
                "{summary}"
            );
            assert!(
                logs.contains("attestation target export rejected"),
                "{logs}"
            );
        }
    }
}

#[tokio::test]
async fn nonce_errors_skip_uploads_and_targets_are_retried_next_cycle() {
    for all_skipped in [false, true] {
        let backend = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let unavailable = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let bmc_port = unavailable.local_addr().unwrap().port();
        drop(unavailable);
        let mut runtime =
            Runtime::start_with_targets(backend.local_addr().unwrap().port(), bmc_port, 1, 6, 6);
        let (mut enrollment, _) = accept_request(&backend).await;
        reply(&mut enrollment, r#"{"jwtAssertion":"example-jwt"}"#).await;
        let inventory_ok = format!(r#"{{"results":[{}]}}"#, [r#"{"status":"OK"}"#; 6].join(","));
        let (mut inventory, _) = accept_request(&backend).await;
        reply(&mut inventory, &inventory_ok).await;
        for _ in 0..6 {
            let (mut nonce, request) = accept_request(&backend).await;
            assert!(request.contains("/nonce "));
            let (status, body) = if request.contains(":01/nonce") && !all_skipped {
                (
                    200,
                    r#"{"nonce":"example-nonce","jwtAssertion":"rotated-jwt"}"#,
                )
            } else if request.contains(":03/nonce") {
                (403, r#"{"error":"forbidden"}"#)
            } else if request.contains(":04/nonce") {
                (503, r#"{"error":"unavailable"}"#)
            } else if request.contains(":05/nonce") {
                (200, "not JSON")
            } else if request.contains(":06/nonce") {
                (200, r#"{"nonce":""}"#)
            } else {
                (404, r#"{"error":"not registered"}"#)
            };
            reply_status(&mut nonce, status, body).await;
        }
        if !all_skipped {
            let (mut upload, request) = accept_request(&backend).await;
            assert!(request.starts_with("POST /v1/agent/oob_nodes/attestation "));
            assert!(request.contains("02:00:00:00:00:01"));
            assert!(
                request
                    .to_ascii_lowercase()
                    .contains("authorization: bearer rotated-jwt")
            );
            for index in 2..=6 {
                assert!(!request.contains(&format!("02:00:00:00:00:{index:02X}")));
            }
            reply(
                &mut upload,
                r#"{"results":[{"bmcMac":"02:00:00:00:00:01","status":"OK"}]}"#,
            )
            .await;
        }
        // An all-skipped collection must not send even an empty attestation batch.
        let (mut inventory, request) = accept_request(&backend).await;
        assert!(request.starts_with("PUT /v1/agent/oob_nodes "));
        reply(&mut inventory, &inventory_ok).await;
        for _ in 0..6 {
            let (mut nonce, request) = accept_request(&backend).await;
            assert!(request.contains("/nonce "));
            reply(&mut nonce, r#"{"nonce":"next-cycle-nonce"}"#).await;
        }
        let (mut upload, request) = accept_request(&backend).await;
        assert!(request.starts_with("POST /v1/agent/oob_nodes/attestation "));
        for index in 1..=6 {
            assert!(request.contains(&format!("02:00:00:00:00:{index:02X}")));
        }
        let results = (1..=6)
            .map(|index| format!(r#"{{"bmcMac":"02:00:00:00:00:{index:02X}","status":"OK"}}"#))
            .collect::<Vec<_>>()
            .join(",");
        reply(&mut upload, &format!(r#"{{"results":[{results}]}}"#)).await;
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
        let summary = logs
            .lines()
            .find(|line| line.contains("attestation backend sync completed with skipped targets"))
            .expect(&logs);
        assert!(
            summary.contains(if all_skipped {
                "skipped_count=6"
            } else {
                "skipped_count=5"
            }),
            "{summary}"
        );
    }
}

#[tokio::test]
async fn nonce_unauthorized_reenrolls_before_retrying() {
    let backend = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let unavailable = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let bmc_port = unavailable.local_addr().unwrap().port();
    drop(unavailable);
    let mut runtime = Runtime::start(backend.local_addr().unwrap().port(), bmc_port);
    let (mut enrollment, _) = accept_request(&backend).await;
    reply(&mut enrollment, r#"{"jwtAssertion":"example-jwt"}"#).await;
    let (mut inventory, _) = accept_request(&backend).await;
    reply(&mut inventory, r#"{"results":[{"status":"OK"}]}"#).await;
    let (mut nonce, request) = accept_request(&backend).await;
    assert!(request.contains("/nonce "));
    reply_status(&mut nonce, 401, r#"{"error":"expired"}"#).await;
    let (mut reenrollment, request) = accept_request(&backend).await;
    assert!(request.starts_with("POST /v1/agent/enroll "));
    reply(&mut reenrollment, r#"{"jwtAssertion":"renewed-jwt"}"#).await;
    finish_attestation(&backend, &mut runtime).await;
}

#[tokio::test]
async fn shared_transport_recovers_jwt_and_accepts_inventory_without_uuid() {
    let backend = TcpListener::bind("127.0.0.1:0").await.expect("backend");
    let unavailable = TcpListener::bind("127.0.0.1:0").await.expect("unused port");
    let bmc_port = unavailable.local_addr().unwrap().port();
    drop(unavailable);
    let mut runtime = Runtime::start(backend.local_addr().unwrap().port(), bmc_port);

    let (mut enrollment, _) = accept_request(&backend).await;
    reply(&mut enrollment, r#"{"jwtAssertion":"first-jwt"}"#).await;
    let (mut inventory, first_request) = accept_request(&backend).await;
    assert!(first_request.starts_with("PUT /v1/agent/oob_nodes "));
    assert!(
        first_request
            .to_ascii_lowercase()
            .contains("authorization: bearer first-jwt")
    );
    reply_status(&mut inventory, 401, r#"{"error":"expired"}"#).await;

    let (mut enrollment, request) = accept_request(&backend).await;
    assert!(request.starts_with("POST /v1/agent/enroll "));
    assert!(
        request
            .to_ascii_lowercase()
            .contains("authorization: bearer example-token")
    );
    reply(&mut enrollment, r#"{"jwtAssertion":"second-jwt"}"#).await;
    let (mut inventory, request) = accept_request(&backend).await;
    assert!(request.starts_with("PUT /v1/agent/oob_nodes "));
    assert!(
        request
            .to_ascii_lowercase()
            .contains("authorization: bearer second-jwt")
    );
    // Every retry must carry the timestamp belonging to its refreshed session.
    let enrolled_at = |request: &str| {
        request
            .split_once("\"enrolledAt\":\"")
            .unwrap()
            .1
            .split('"')
            .next()
            .unwrap()
            .to_string()
    };
    assert_ne!(enrolled_at(&first_request), enrolled_at(&request));
    reply(
        &mut inventory,
        r#"{"results":[{"status":"OK","computeZone":"example-zone"}]}"#,
    )
    .await;

    let (mut nonce, request) = accept_request(&backend).await;
    assert!(request.starts_with("POST /v1/agent/oob_nodes/02:00:00:00:00:01/nonce "));
    assert!(
        request
            .to_ascii_lowercase()
            .contains("authorization: bearer second-jwt")
    );
    reply(&mut nonce, r#"{"nonce":"example-nonce","jwtAssertion":"rotated-jwt","nonceRefreshTimestamp":"2026-01-01T00:00:00Z"}"#).await;
    let (mut upload, request) = accept_request(&backend).await;
    assert!(request.starts_with("POST /v1/agent/oob_nodes/attestation "));
    assert!(
        request
            .to_ascii_lowercase()
            .contains("authorization: bearer rotated-jwt")
    );
    assert!(request.contains("02:00:00:00:00:01"));
    reply(
        &mut upload,
        r#"{"results":[{"bmcMac":"02:00:00:00:00:01","status":"OK"}]}"#,
    )
    .await;
    runtime.signal("-TERM").await;
    runtime.expect_clean_exit().await;
}

#[test]
fn runtime_help_shows_token_environment_name_without_its_value() {
    let output = std::process::Command::new(env!("CARGO_BIN_EXE_bmc-collector"))
        .args(["run", "--help"])
        .env("FLEETINT_AGENT_TOKEN", "example-private-token")
        .output()
        .expect("CLI help");
    assert!(output.status.success());
    let help = String::from_utf8(output.stdout).unwrap();
    assert!(help.contains("FLEETINT_AGENT_TOKEN"));
    assert!(!help.contains("example-private-token"));
}
