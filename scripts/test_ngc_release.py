# Copyright (c) 2026, NVIDIA CORPORATION & AFFILIATES. All rights reserved.
# SPDX-License-Identifier: Apache-2.0

"""Exercise release destination validation and render the actual NGC chart package."""

import importlib.util
from pathlib import Path
import shutil
import subprocess
import tempfile
import unittest

ROOT = Path(__file__).resolve().parents[1]
SPEC = importlib.util.spec_from_file_location("ngc_release", ROOT / "scripts/prepare-ngc-release.py")
RELEASE = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(RELEASE)


class ConfigurationTests(unittest.TestCase):
    def settings(self, **changes):
        settings = {
            "RELEASE_VERSION": "v1.2.3-rc.1",
            "NGC_ORG": "1234567890",
            "NGC_TEAM": "example-team",
            "NGC_PUBLIC_IMAGE_PREFIX": "nvcr.io/nvidia/example-team",
        }
        settings.update(changes)
        return settings

    def test_staging_and_public_destinations_are_distinct(self):
        config = RELEASE.configuration(self.settings())
        self.assertEqual(config["image"], "nvcr.io/1234567890/example-team/fleet-intelligence-oob-collector")
        self.assertEqual(config["nico_image"], "nvcr.io/1234567890/example-team/nico-hardware-health")
        self.assertEqual(config["chart_target"], "1234567890/example-team/fleet-intelligence-oob-collector")
        self.assertEqual(config["version"], "1.2.3-rc.1")
        self.assertEqual(config["tag"], "v1.2.3-rc.1")

    def test_stable_versions(self):
        self.assertEqual(RELEASE.configuration(self.settings(RELEASE_VERSION="v0.2.0"))["version"], "0.2.0")

    def test_missing_configuration_blocks_publication(self):
        for key in ("RELEASE_VERSION", "NGC_ORG", "NGC_TEAM", "NGC_PUBLIC_IMAGE_PREFIX"):
            with self.subTest(key=key), self.assertRaises(ValueError):
                RELEASE.configuration(self.settings(**{key: ""}))

    def test_ci_cannot_target_public_organization(self):
        with self.assertRaises(ValueError):
            RELEASE.configuration(self.settings(NGC_ORG="nvidia"))

    def test_invalid_versions_and_output_injection(self):
        for tag in ("1.2.3", "v01.2.3", "v1.2.3-01", "v1.2.3-", "v1.2", "v1.2.3\nimage=evil", "v1.2.3;echo bad", "v1.2.3+build"):
            with self.subTest(tag=tag), self.assertRaises(ValueError):
                RELEASE.configuration(self.settings(RELEASE_VERSION=tag))

    def test_invalid_registry_and_team_paths(self):
        for key, value in (("NGC_ORG", "nvidia/team"), ("NGC_TEAM", "../public"), ("NGC_TEAM", "team\nimage=bad"), ("NGC_PUBLIC_IMAGE_PREFIX", "ghcr.io/nvidia/team"), ("NGC_PUBLIC_IMAGE_PREFIX", "nvcr.io/private/team"), ("NGC_PUBLIC_IMAGE_PREFIX", "nvcr.io/nvidia/team:latest")):
            with self.subTest(key=key, value=value), self.assertRaises(ValueError):
                RELEASE.configuration(self.settings(**{key: value}))


@unittest.skipUnless(shutil.which("helm"), "Helm is required to test packaged release charts")
class ChartTests(unittest.TestCase):
    def test_packaged_chart_uses_promoted_images_in_all_workloads(self):
        source = ROOT / "deployments/helm/fleet-intelligence-oob-collector/values.yaml"
        original = source.read_text()
        with tempfile.TemporaryDirectory() as directory:
            work = Path(directory)
            chart = work / "chart"
            RELEASE.prepare_chart("nvcr.io/nvidia/example-team", chart)
            self.assertEqual((chart / "LICENSE").read_bytes(), (ROOT / "LICENSE").read_bytes())
            subprocess.run(["helm", "package", str(chart), "--version", "1.2.3-rc.1", "--app-version", "1.2.3-rc.1", "--destination", str(work)], check=True, capture_output=True, text=True)
            package = work / "fleet-intelligence-oob-collector-1.2.3-rc.1.tgz"
            rendered = subprocess.run(["helm", "template", "test", str(package), "--values", str(ROOT / "deployments/helm/fleet-intelligence-oob-collector/ci/test-values.yaml")], check=True, capture_output=True, text=True).stdout
            self.assertIn("nvcr.io/nvidia/example-team/fleet-intelligence-oob-collector:1.2.3-rc.1", rendered)
            self.assertIn("nvcr.io/nvidia/example-team/nico-hardware-health:1.2.3-rc.1", rendered)
            self.assertNotIn("ghcr.io/dsx-ai-factory/", rendered)
            self.assertNotIn("nvcr.io/1234567890/", rendered)
            # The independently released gateway is not published by this repository.
            self.assertIn("ghcr.io/nvidia/fleetint-otelcol:1.5.1", rendered)
        self.assertEqual(source.read_text(), original)

    def test_changed_source_defaults_require_explicit_review(self):
        with tempfile.TemporaryDirectory() as directory:
            work = Path(directory)
            root = work / "source"
            chart = root / "deployments/helm/fleet-intelligence-oob-collector"
            chart.mkdir(parents=True)
            (chart / "values.yaml").write_text("bmcCollector: {}\n")
            with self.assertRaisesRegex(ValueError, "Expected exactly one"):
                RELEASE.prepare_chart("nvcr.io/nvidia/example-team", work / "chart", root)


if __name__ == "__main__":
    unittest.main()
