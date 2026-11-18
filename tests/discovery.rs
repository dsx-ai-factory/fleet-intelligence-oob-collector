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
        std::fs::write(&config, "[credentials]\nfile='missing-credentials.json'\n[endpoint_source]\ntype='nico'\napi_url='https://nico.example.com/v2'\norg='example'\nsite_id='site'\ntoken_file='missing-token'\n").unwrap();
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
    std::fs::write(&config, "[credentials]\nfile='missing-credentials.json'\n[[targets]]\nip='192.0.2.10'\nmac='02:00:00:00:00:01'\nname='fixture-host'\ncredentials_profile='bmc'\n").unwrap();
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
    assert!(!output.contains("password"));
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
