# Copyright (c) 2026, NVIDIA CORPORATION & AFFILIATES. All rights reserved.
# SPDX-License-Identifier: Apache-2.0

"""Verify NICo collection controls survive Helm rendering and trigger a rollout."""

import json
from pathlib import Path
import re
import subprocess
import textwrap
import tomllib
import unittest


ROOT = Path(__file__).resolve().parents[1]
CHART = ROOT / "deployments/helm/fleet-intelligence-oob-collector"
DEFAULT_NICO_SETTINGS = {
    "bmc_request_concurrency": 2,
    "sensor_fetch_interval_seconds": 60,
    "discovery_refresh_interval_seconds": 3600,
    "logs_mode": "auto",
    "logs_auto_retry_sse_after_downgrade": True,
    "reachability_enabled": True,
    "reachability_interval_seconds": 60,
}


def render(settings, template="configmap.yaml", workload=None, *, use_ci_values=True):
    """Render one chart template with explicit NICo setting overrides."""
    command = [
        "helm", "template", "test", str(CHART),
        "--show-only", f"templates/{template}",
    ]
    if use_ci_values:
        command.extend(["--values", str(CHART / "ci/test-values.yaml")])
    else:
        targets = [{"address": "192.0.2.10", "mac": "02:00:00:00:00:01"}]
        command.extend(["--set-json", f"inventory.targets={json.dumps(targets)}"])
    for key, value in settings.items():
        command.extend(["--set-json", f"nicoHardwareHealth.config.{key}={json.dumps(value)}"])
    for key, value in (workload or {}).items():
        command.extend(["--set-json", f"nicoHardwareHealth.{key}={json.dumps(value)}"])
    return subprocess.run(command, capture_output=True, text=True, check=False)


class NicoHelmTests(unittest.TestCase):
    def test_bundle_defaults_without_collection_overrides(self):
        result = render({}, use_ci_values=False)
        self.assertEqual(result.returncode, 0, result.stderr)
        config = tomllib.loads(textwrap.dedent(result.stdout.split("  collector.toml: |\n", 1)[1]))
        self.assertEqual(config["nico_health"], DEFAULT_NICO_SETTINGS)
        result = render({}, "bmc-collector-deployment.yaml", use_ci_values=False)
        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertRegex(result.stdout, r'- --bmc-max-in-flight\s+- "2"')
        self.assertIn("- --insecure-tls", result.stdout)
        self.assertRegex(result.stdout, r'- name: FLEETINT_NICO_LOG_LEVEL\s+value: "warn"')
        result = render({}, "nico-hardware-health-deployment.yaml", use_ci_values=False)
        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertRegex(result.stdout, r'- name: RUST_LOG\s+value: "warn"')

    def test_collector_bmc_certificate_verification_override(self):
        result = subprocess.run([
            "helm", "template", "test", str(CHART),
            "--values", str(CHART / "ci/test-values.yaml"),
            "--show-only", "templates/bmc-collector-deployment.yaml",
            "--set", "bmcCollector.redfish.insecureTls=false",
        ], capture_output=True, text=True, check=False)
        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertNotIn("--insecure-tls", result.stdout)

    def test_optional_settings_render_as_top_level_toml(self):
        """Preserve independent overrides, defaults, and shared target configuration."""
        for interval, concurrency in [(None, None), (120, None), (None, 2), (300, 1), (4294967295, 4294967295)]:
            with self.subTest(interval=interval, concurrency=concurrency):
                result = render({
                    "sensorFetchIntervalSeconds": interval,
                    "bmcRequestConcurrency": concurrency,
                })
                self.assertEqual(result.returncode, 0, result.stderr)
                contents = textwrap.dedent(result.stdout.split("  collector.toml: |\n", 1)[1])
                config = tomllib.loads(contents)
                expected = {
                    key: value for key, value in DEFAULT_NICO_SETTINGS.items()
                    if key not in ("sensor_fetch_interval_seconds", "bmc_request_concurrency")
                }
                if interval is not None:
                    expected["sensor_fetch_interval_seconds"] = interval
                if concurrency is not None:
                    expected["bmc_request_concurrency"] = concurrency
                self.assertEqual(config.get("nico_health", {}), expected)
                self.assertEqual(len(config["targets"]), 3)
                self.assertEqual(config["targets"][0]["credentials_profile"], "bmc")
                self.assertEqual(config["credentials"]["file"], "/run/secrets/bmc/credentials.json")

    def test_all_flat_controls_preserve_names_values_and_placement(self):
        """Pass every typed setting through Helm, including false, zero, and []."""
        source = tomllib.loads((ROOT / "crates/bmc-inventory/tests/fixtures/nico-health-settings.toml").read_text())

        settings = {
            re.sub(r"_([a-z])", lambda match: match[1].upper(), key): value
            for key, value in source.items()
        }
        result = render(settings)
        self.assertEqual(result.returncode, 0, result.stderr)
        config = tomllib.loads(textwrap.dedent(result.stdout.split("  collector.toml: |\n", 1)[1]))
        self.assertEqual(config["nico_health"], source)
        self.assertEqual(len(config["targets"]), 3)
        self.assertEqual(config["credentials"]["file"], "/run/secrets/bmc/credentials.json")

    def test_log_mode_and_recovery_overrides(self):
        """Preserve explicit modes and false recovery; null delegates to native defaults."""
        for mode, recovery in [("sse", False), ("periodic", True), ("auto", False), (None, None)]:
            with self.subTest(mode=mode, recovery=recovery):
                result = render({
                    "logsMode": mode,
                    "logsAutoRetrySseAfterDowngrade": recovery,
                })
                self.assertEqual(result.returncode, 0, result.stderr)
                config = tomllib.loads(textwrap.dedent(result.stdout.split("  collector.toml: |\n", 1)[1]))
                settings = config["nico_health"]
                if mode is None:
                    self.assertNotIn("logs_mode", settings)
                    self.assertNotIn("logs_auto_retry_sse_after_downgrade", settings)
                else:
                    self.assertEqual(settings["logs_mode"], mode)
                    self.assertEqual(settings["logs_auto_retry_sse_after_downgrade"], recovery)

    def test_flat_validation_and_rollout(self):
        """Reject invalid flat settings and roll NICo after a flat control edit."""
        for key, value in [
            ("leakDetectorEnabled", "false"),
            ("leakDetectorPollIntervalSeconds", 0),
            ("logsMode", "unsupported"),
            ("logsSseInitialBackoffSeconds", -1),
            ("logsAutoConnectFailureThreshold", 0),
            ("logsExcludeServices", [False]),
            ("reachabilityLogMode", "invalid"),
            ("rateLimitMaxJitterMilliseconds", -1),
            ("discoveryConcurrency", 4294967296),
            ("unknownControl", True),
        ]:
            with self.subTest(key=key, value=value):
                self.assertNotEqual(render({key: value}).returncode, 0)
        for settings in [
            {"logsSseInitialBackoffSeconds": 31},
            {"logsSseInitialBackoffSeconds": 10, "logsSseMaxBackoffSeconds": 9},
        ]:
            result = render(settings)
            self.assertNotEqual(result.returncode, 0)
            self.assertIn("logsSseMaxBackoffSeconds must be at least", result.stderr)
        for settings in [
            {"logsSseMaxBackoffSeconds": 1},
            {"logsSseInitialBackoffSeconds": 60, "logsSseMaxBackoffSeconds": 60},
        ]:
            result = render(settings)
            self.assertEqual(result.returncode, 0, result.stderr)
        checksums = []
        for settings in [
            {}, {"leakDetectorEnabled": False}, {"logsSseMaxBackoffSeconds": 60},
            {"logsMode": "sse"}, {"logsAutoRetrySseAfterDowngrade": False},
        ]:
            result = render(settings, "nico-hardware-health-deployment.yaml")
            self.assertEqual(result.returncode, 0, result.stderr)
            checksums.append(re.search(r"checksum/config: ([0-9a-f]{64})", result.stdout)[1])
        self.assertEqual(len(set(checksums)), 5)

    def test_collection_settings_are_separate_from_workload_settings(self):
        """Validate both boundaries and preserve component reporting metadata."""
        self.assertNotEqual(render({"image": {"tag": "test"}}).returncode, 0)
        self.assertNotEqual(render({}, workload={"logsMode": "sse"}).returncode, 0)
        self.assertNotEqual(render({"enabled": False}).returncode, 0)
        result = render({"logsMode": "sse"}, "nico-hardware-health-deployment.yaml",
                        workload={"image.tag": "config-boundary-test", "logLevel": "debug"})
        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertIn("nico-hardware-health:config-boundary-test", result.stdout)
        self.assertIn('value: "debug"', result.stdout)
        result = render({}, "bmc-collector-deployment.yaml")
        self.assertEqual(result.returncode, 0, result.stderr)
        for name in ["FLEETINT_NICO_ENABLED", "FLEETINT_NICO_LOG_LEVEL", "FLEETINT_NICO_TELEMETRY_DESTINATION"]:
            self.assertIn(name, result.stdout)
        result = render({}, workload={"config": None})
        self.assertEqual(result.returncode, 0, result.stderr)
        config = tomllib.loads(textwrap.dedent(result.stdout.split("  collector.toml: |\n", 1)[1]))
        self.assertNotIn("nico_health", config)

    def test_invalid_settings_fail_schema_validation(self):
        """Reject invalid types and values before producing a ConfigMap."""
        for key in ["sensorFetchIntervalSeconds", "bmcRequestConcurrency"]:
            for value in [0, -1, 1.5, "120", True, 4294967296]:
                with self.subTest(key=key, value=value):
                    result = render({key: value})
                    self.assertNotEqual(result.returncode, 0)
                    self.assertIn(key, result.stderr)

    def test_either_setting_changes_nico_pod_checksum(self):
        """Restart NICo when either collection setting changes."""
        checksums = []
        for settings in [{}, {"sensorFetchIntervalSeconds": 300}, {"bmcRequestConcurrency": 1}]:
            result = render(settings, "nico-hardware-health-deployment.yaml")
            self.assertEqual(result.returncode, 0, result.stderr)
            checksum = re.search(r"checksum/config: ([0-9a-f]{64})", result.stdout)
            self.assertIsNotNone(checksum)
            checksums.append(checksum.group(1))
        self.assertEqual(len(set(checksums)), 3)


if __name__ == "__main__":
    unittest.main()
