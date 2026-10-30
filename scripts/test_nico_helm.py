# Copyright (c) 2026, NVIDIA CORPORATION & AFFILIATES. All rights reserved.
# SPDX-License-Identifier: Apache-2.0

"""Verify NICo load controls survive Helm rendering and trigger a rollout."""

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
                self.assertEqual(config.get("nico", {}), expected)
                self.assertEqual(len(config["targets"]), 3)
                self.assertEqual(config["targets"][0]["credentials_profile"], "bmc")
                self.assertEqual(config["credentials"]["file"], "/run/secrets/bmc/credentials.json")
                if not expected:
                    self.assertNotIn("nico", config)

    def test_invalid_settings_fail_schema_validation(self):
        for key in ["sensorFetchIntervalSeconds", "bmcRequestConcurrency"]:
            for value in [0, -1, 1.5, "120", True, 4294967296]:
                with self.subTest(key=key, value=value):
                    result = render({key: value})
                    self.assertNotEqual(result.returncode, 0)
                    self.assertIn(key, result.stderr)

    def test_either_setting_changes_nico_pod_checksum(self):
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
