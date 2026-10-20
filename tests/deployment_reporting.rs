// Copyright (c) 2026, NVIDIA CORPORATION & AFFILIATES. All rights reserved.
// SPDX-License-Identifier: Apache-2.0

use std::process::Command;
use std::time::{SystemTime, UNIX_EPOCH};

use bmc_inventory::config::DeploymentReport;

fn render(template: &str, overrides: &[&str]) -> Option<String> {
    let mut command = Command::new("helm");
    command.current_dir(env!("CARGO_MANIFEST_DIR")).args([
        "template",
        "report-test",
        "deployments/helm/fleet-intelligence-oob-collector",
        "--show-only",
        template,
    ]);
    for value in overrides {
        command.args(["--set", value]);
    }
    let output = match command.output() {
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => {
            eprintln!("Helm is not installed; skipping deployment reporting integration test");
            return None;
        }
        result => result.expect("run Helm"),
    };
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    Some(String::from_utf8(output.stdout).unwrap())
}

fn parse_report(yaml: &str) -> DeploymentReport {
    let (_, block) = yaml.split_once("  deployment.json: |\n").unwrap();
    let json = block
        .lines()
        .take_while(|line| line.starts_with("    "))
        .map(|line| &line[4..])
        .collect::<Vec<_>>()
        .join("\n");
    let path = std::env::temp_dir().join(format!(
        "collector-report-{}-{}.json",
        std::process::id(),
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    std::fs::write(&path, &json).unwrap();
    let report = DeploymentReport::load(path.to_str().unwrap());
    std::fs::remove_file(path).unwrap();
    report.expect("Helm output must match the collector reporting schema")
}

#[test]
fn helm_report_matches_defaults_and_is_mounted_with_rollout_checksum() {
    let Some(yaml) = render("templates/collector-deployment-configmap.yaml", &[]) else {
        return;
    };
    let report = parse_report(&yaml);
    let nico = report.nico.unwrap();
    let telemetry = report.telemetry.unwrap();
    assert!(nico.enabled);
    assert_eq!(nico.metrics_prefix, "fleetint");
    assert_eq!(nico.metrics_port, 9009);
    assert!(!nico.tls_enabled);
    assert!(telemetry.enabled);
    assert_eq!(telemetry.grpc_port, 4317);
    assert_eq!(telemetry.memory_limiter.limit_percentage, 80);
    assert!(!yaml.contains("flushInterval"));
    assert!(!yaml.contains("batchSize"));
    assert!(!yaml.contains("endpoint"));
    let deployment = render("templates/bmc-collector-deployment.yaml", &[]).unwrap();
    assert!(deployment.contains("checksum/deployment-config:"));
    assert!(deployment.contains("--deployment-config"));
    assert!(deployment.contains("/etc/fleet-intelligence-reporting/deployment.json"));
    assert!(deployment.contains("report-test-fleet-intelligence-oob-collector-reporting"));
}

#[test]
fn helm_report_tracks_overrides_and_never_includes_private_settings() {
    let overrides = [
        "nicoHardwareHealth.enabled=false",
        "nicoHardwareHealth.image.tag=nico-test",
        "nicoHardwareHealth.metricsPrefix=custom",
        "nicoHardwareHealth.service.port=9010",
        "nicoHardwareHealth.service.enabled=false",
        "nicoHardwareHealth.tlsSecretName=private-secret-name",
        "nicoHardwareHealth.otlpEndpoint=http://private.example.com:4317",
        "otelGateway.enabled=false",
        "otelGateway.image.tag=gateway-test",
        "otelGateway.replicas=2",
        "otelGateway.service.grpcPort=5317",
        "otelGateway.memoryLimiter.limitPercentage=70",
        "otelGateway.networkPolicy.enabled=false",
        "bmcCollector.backendUrl=https://private.example.com",
        "bmcCollector.agentTokenSecretName=private-secret-name",
    ];
    let Some(yaml) = render("templates/collector-deployment-configmap.yaml", &overrides) else {
        return;
    };
    assert!(!yaml.contains("private.example.com"));
    assert!(!yaml.contains("private-secret-name"));
    let report = parse_report(&yaml);
    let nico = report.nico.unwrap();
    let telemetry = report.telemetry.unwrap();
    assert!(!nico.enabled);
    assert_eq!(nico.image_tag, "nico-test");
    assert_eq!(nico.metrics_prefix, "custom");
    assert_eq!(nico.metrics_port, 9010);
    assert!(!nico.service_enabled);
    assert!(nico.tls_enabled);
    assert!(!telemetry.enabled);
    assert_eq!(telemetry.image_tag, "gateway-test");
    assert_eq!(telemetry.replicas, 2);
    assert_eq!(telemetry.grpc_port, 5317);
    assert!(!telemetry.network_policy_enabled);
    assert_eq!(telemetry.memory_limiter.limit_percentage, 70);
    let original = render("templates/bmc-collector-deployment.yaml", &[]).unwrap();
    let changed = render("templates/bmc-collector-deployment.yaml", &overrides).unwrap();
    let checksum = |yaml: String| {
        yaml.lines()
            .find(|line| line.contains("checksum/deployment-config:"))
            .unwrap()
            .trim()
            .to_string()
    };
    assert_ne!(checksum(original), checksum(changed));
}
