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


def render(settings, template="configmap.yaml"):
    """Render one chart template with explicit NICo setting overrides."""
    command = [
        "helm", "template", "test", str(CHART),
        "--values", str(CHART / "ci/test-values.yaml"),
        "--show-only", f"templates/{template}",
    ]
    for key, value in settings.items():
        command.extend(["--set-json", f"nicoHardwareHealth.{key}={json.dumps(value)}"])
    return subprocess.run(command, capture_output=True, text=True, check=False)


class NicoHelmTests(unittest.TestCase):
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
                expected = {}
                if interval is not None:
                    expected["sensor_fetch_interval_seconds"] = interval
                if concurrency is not None:
                    expected["bmc_request_concurrency"] = concurrency
                self.assertEqual(config.get("nico_health", {}), expected)
                self.assertEqual(len(config["targets"]), 3)
                self.assertEqual(config["targets"][0]["credentials_profile"], "bmc")
                self.assertEqual(config["credentials"]["file"], "/run/secrets/bmc/credentials.json")
                if not expected:
                    self.assertNotIn("nico_health", config)

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
        checksums = []
        for settings in [{}, {"leakDetectorEnabled": False}, {"logsSseMaxBackoffSeconds": 60}]:
            result = render(settings, "nico-hardware-health-deployment.yaml")
            self.assertEqual(result.returncode, 0, result.stderr)
            checksums.append(re.search(r"checksum/config: ([0-9a-f]{64})", result.stdout)[1])
        self.assertEqual(len(set(checksums)), 3)

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
