// Copyright (c) 2026, NVIDIA CORPORATION & AFFILIATES. All rights reserved.
// SPDX-License-Identifier: Apache-2.0

use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering};
use tokio::process::Command;

struct ConfigDirectory(PathBuf);
impl ConfigDirectory {
    fn new() -> Self {
        static NEXT: AtomicU64 = AtomicU64::new(0);
        let directory = std::env::temp_dir().join(format!(
            "fleetint-discover-cli-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        std::fs::create_dir_all(&directory).unwrap();
        Self(directory)
    }
    fn nico_config(&self) -> PathBuf {
        let config = self.0.join("config.toml");
        std::fs::write(&config, "[credentials]\nfile='missing-credentials.json'\n[endpoint_source]\ntype='nico'\napi_url='https://nico.example.com/v2'\norg='example'\ntoken_url='https://idp.example.com/token'\nclient_id_file='missing-id'\nclient_secret_file='missing-secret'\n").unwrap();
        config
    }
}
impl Drop for ConfigDirectory {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

#[tokio::test]
async fn discover_needs_neither_passwords_nor_hardware_access() {
    let directory = ConfigDirectory::new();
    let config = directory.0.join("config.toml");
    std::fs::write(&config, "node_group='configured-group'\n[credentials]\nfile='missing-credentials.json'\n[[targets]]\nip='192.0.2.10'\nmac='02:00:00:00:00:01'\nname='fixture-host'\ncredentials_profile='bmc'\n").unwrap();
    let output = Command::new(env!("CARGO_BIN_EXE_bmc-collector"))
        .arg("discover")
        .arg("--config")
        .arg(config)
        .output()
        .await
        .unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let output = String::from_utf8(output.stdout).unwrap();
    assert!(output.contains("192.0.2.10") && output.contains("02:00:00:00:00:01"));
    assert!(output.contains("fixture-host"));
    // Discover output shows the effective placement that collection will use.
    assert!(output.contains("\"node_group\": \"configured-group\""));
    assert!(!output.contains("password"));
}

#[cfg(unix)]
#[tokio::test]
async fn startup_retries_unavailable_discovery_and_stops_cleanly() {
    use tokio::io::{AsyncBufReadExt, BufReader};

    let directory = ConfigDirectory::new();
    // Bind and release a port so the token endpoint refuses connections.
    let closed = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    let port = closed.local_addr().unwrap().port();
    drop(closed);
    std::fs::write(directory.0.join("client_id"), "fixture-client").unwrap();
    std::fs::write(directory.0.join("client_secret"), "fixture-secret").unwrap();
    let config = directory.0.join("config.toml");
    std::fs::write(&config, format!("[credentials]\nfile='missing-credentials.json'\n[endpoint_source]\ntype='nico'\napi_url='https://127.0.0.1:{port}/v2'\norg='example'\ntoken_url='https://127.0.0.1:{port}/token'\nclient_id_file='client_id'\nclient_secret_file='client_secret'\ncredentials_profile='bmc'\n")).unwrap();
    let mut child = Command::new(env!("CARGO_BIN_EXE_bmc-collector"))
        .args(["run", "--config"])
        .arg(&config)
        .env("FLEETINT_BACKEND_URL", "http://127.0.0.1:9")
        .env("FLEETINT_ENROLLMENT_TOKEN", "example-token")
        .env("NO_PROXY", "*")
        .env("NO_COLOR", "1")
        .env("RUST_LOG", "info")
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::piped())
        .kill_on_drop(true)
        .spawn()
        .unwrap();
    let mut lines = BufReader::new(child.stderr.take().unwrap()).lines();
    tokio::time::timeout(std::time::Duration::from_secs(10), async {
        while let Some(line) = lines.next_line().await.unwrap() {
            if line.contains("initial endpoint discovery failed; retrying") {
                return;
            }
        }
        panic!("collector exited instead of retrying discovery");
    })
    .await
    .expect("retry log");
    assert!(child.try_wait().unwrap().is_none(), "collector exited");
    let pid = child.id().unwrap().to_string();
    assert!(
        Command::new("kill")
            .args(["-TERM", &pid])
            .status()
            .await
            .unwrap()
            .success()
    );
    let status = tokio::time::timeout(std::time::Duration::from_secs(5), child.wait())
        .await
        .expect("collector should stop promptly")
        .unwrap();
    assert!(status.success(), "{status}");
}

#[tokio::test]
async fn renderer_rejects_discovery_config_before_writing_or_contacting_the_api() {
    let directory = ConfigDirectory::new();
    let output_path = directory.0.join("health.toml");
    let result = Command::new(env!("CARGO_BIN_EXE_bmc-collector"))
        .arg("render-nico-config")
        .arg("--config")
        .arg(directory.nico_config())
        .arg("--output")
        .arg(&output_path)
        .output()
        .await
        .unwrap();
    assert!(!result.status.success());
    assert!(String::from_utf8_lossy(&result.stderr).contains("static targets only"));
    assert!(!output_path.exists());
}
