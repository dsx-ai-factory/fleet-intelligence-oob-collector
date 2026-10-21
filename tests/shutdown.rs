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
}

impl Runtime {
    fn start(backend_port: u16, bmc_port: u16) -> Self {
        let config = std::env::temp_dir().join(format!(
            "fleetint-shutdown-{}-{}.toml",
            std::process::id(),
            NEXT_CONFIG.fetch_add(1, Ordering::Relaxed)
        ));
        std::fs::write(
            &config,
            format!(
                "[[targets]]\nip = \"127.0.0.1\"\nport = {bmc_port}\nmac = \"02:00:00:00:00:01\"\nusername = \"example-user\"\npassword = \"example-password\"\n"
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
            .env(
                "FLEETINT_BACKEND_URL",
                format!("http://127.0.0.1:{backend_port}"),
            )
            .env("FLEETINT_AGENT_TOKEN", "example-token")
            .env("NO_PROXY", "*")
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .kill_on_drop(true)
            .spawn()
            .expect("start collector");
        Self { child, config }
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
    stream
        .write_all(
            format!(
                "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
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
