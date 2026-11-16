# Copyright (c) 2026, NVIDIA CORPORATION & AFFILIATES. All rights reserved.
# SPDX-License-Identifier: Apache-2.0

"""Unit tests for the external NICo Deployment integration command."""

import copy
import importlib.util
from pathlib import Path
import unittest


SCRIPT = Path(__file__).with_name("integrate_external_nico.py")
SPEC = importlib.util.spec_from_file_location("integrate_external_nico", SCRIPT)
MODULE = importlib.util.module_from_spec(SPEC)
assert SPEC.loader is not None
SPEC.loader.exec_module(MODULE)


class ExternalNicoIntegrationTests(unittest.TestCase):
    def test_missing_named_container_is_rejected(self):
        with self.assertRaisesRegex(SystemExit, "expected one container"):
            MODULE.mutate_template(
                {"metadata": {}, "spec": {"containers": []}},
                "health",
                {
                    "sourceLabel": "release",
                    "configChecksum": "checksum",
                    "qualifiedNicoRevision": "revision",
                    "qualifiedNicoImage": "registry.example/nico:qualified",
                    "expectedSourceNicoImage": "registry.example/nico:official",
                },
            )

    def test_image_transition_accepts_source_and_qualified_images_only(self):
        for image in [
            "registry.example/nico:official",
            "registry.example/nico:qualified",
        ]:
            MODULE.validate_image_transition(
                image,
                "registry.example/nico:official",
                "registry.example/nico:qualified",
            )
        with self.assertRaisesRegex(SystemExit, "matches neither"):
            MODULE.validate_image_transition(
                "registry.example/nico:unexpected",
                "registry.example/nico:official",
                "registry.example/nico:qualified",
            )

    def test_mutation_preserves_existing_config_and_customer_initializer(self):
        template = {
            "metadata": {"labels": {}, "annotations": {}},
            "spec": {
                "containers": [
                    {
                        "name": "health",
                        "image": "registry.example/nico:official",
                        "args": [
                            "--customer-mode",
                            "safe",
                            "/etc/customer-nico/config.toml",
                        ],
                        "volumeMounts": [
                            {
                                "name": "customer-config",
                                "mountPath": "/etc/customer-nico",
                                "readOnly": True,
                            },
                            {"name": "customer-data", "mountPath": "/var/lib/customer"},
                            {"name": "customer-tmp", "mountPath": "/tmp"},
                        ],
                    }
                ],
                "initContainers": [
                    {
                        "name": "customer-config-writer",
                        "image": "registry.example/writer:1",
                        "volumeMounts": [
                            {
                                "name": "customer-config",
                                "mountPath": "/etc/customer-nico",
                            }
                        ],
                    }
                ],
                "volumes": [
                    {"name": "customer-config", "emptyDir": {}},
                    {"name": "customer-data", "emptyDir": {}},
                    {"name": "customer-tmp", "emptyDir": {}},
                ],
            },
        }
        integration = {
            "sourceLabel": "fleet-intelligence-oob-collector",
            "configChecksum": "abc123",
            "qualifiedNicoRevision": "qualified-revision",
            "qualifiedNicoImage": "registry.example/nico:qualified",
            "expectedSourceNicoImage": "registry.example/nico:official",
            "initContainer": {
                "name": "fleet-intelligence-nico-config",
                "image": "registry.example/collector:1",
                "args": [
                    "render-nico-config",
                    "--base-config",
                    MODULE.BASE_CONFIG_PLACEHOLDER,
                    "--output",
                    "/etc/nico-health/config.toml",
                ],
                "volumeMounts": [
                    {
                        "name": "fleet-config",
                        "mountPath": "/etc/fleet-intelligence",
                        "readOnly": True,
                    },
                    {
                        "name": "fleet-intelligence-nico-config",
                        "mountPath": "/etc/nico-health",
                    }
                ],
            },
            "mainContainer": {
                "args": ["/etc/nico-health/config.toml"],
                "env": [],
                "volumeMounts": [
                    {
                        "name": "fleet-intelligence-nico-config",
                        "mountPath": "/etc/nico-health",
                        "readOnly": True,
                    },
                    {
                        "name": "fleet-intelligence-nico-tmp",
                        "mountPath": "/tmp",
                    },
                ],
            },
            "volumes": [
                {"name": "fleet-intelligence-nico-config", "emptyDir": {}},
                {"name": "fleet-intelligence-nico-tmp", "emptyDir": {}},
            ],
        }

        result = MODULE.mutate_template(
            copy.deepcopy(template),
            "health",
            integration,
            "/etc/customer-nico/config.toml",
        )
        initializers = result["spec"]["initContainers"]
        self.assertEqual(
            [container["name"] for container in initializers],
            ["customer-config-writer", "fleet-intelligence-nico-config"],
        )
        fi_initializer = initializers[-1]
        base_index = fi_initializer["args"].index("--base-config") + 1
        self.assertEqual(
            fi_initializer["args"][base_index],
            "/etc/fleet-intelligence/external-nico-base/config.toml",
        )
        self.assertIn(
            {
                "name": "customer-config",
                "mountPath": "/etc/fleet-intelligence/external-nico-base",
                "readOnly": True,
            },
            fi_initializer["volumeMounts"],
        )
        health = result["spec"]["containers"][0]
        self.assertEqual(
            health["args"],
            ["--customer-mode", "safe", "/etc/nico-health/config.toml"],
        )
        self.assertEqual(
            {mount["name"] for mount in health["volumeMounts"]},
            {
                "customer-config",
                "customer-data",
                "customer-tmp",
                "fleet-intelligence-nico-config",
            },
        )
        self.assertNotIn(
            "fleet-intelligence-nico-tmp",
            {volume["name"] for volume in result["spec"]["volumes"]},
        )
        self.assertEqual(
            result["metadata"]["annotations"][MODULE.BASE_CONFIG_PATH_ANNOTATION],
            "/etc/customer-nico/config.toml",
        )

        rerun = MODULE.mutate_template(
            copy.deepcopy(result),
            "health",
            integration,
            "/etc/customer-nico/config.toml",
        )
        self.assertEqual(
            [container["name"] for container in rerun["spec"]["initContainers"]],
            ["customer-config-writer", "fleet-intelligence-nico-config"],
        )
        rerun_fi = rerun["spec"]["initContainers"][-1]
        self.assertEqual(
            rerun_fi["args"][rerun_fi["args"].index("--base-config") + 1],
            "/etc/fleet-intelligence/external-nico-base/config.toml",
        )

    def test_integration_requires_a_mounted_absolute_config_path(self):
        container = {
            "name": "health",
            "volumeMounts": [{"name": "config", "mountPath": "/etc/nico"}],
        }
        with self.assertRaisesRegex(SystemExit, "must be absolute"):
            MODULE.config_source_from_main_container(container, "config.toml")
        with self.assertRaisesRegex(SystemExit, "not covered by a volume mount"):
            MODULE.config_source_from_main_container(container, "/other/config.toml")
        self.assertTrue(
            MODULE.mount_paths_overlap(
                "/etc/nico-health/config.toml", "/etc/nico-health"
            )
        )

        mount, path = MODULE.config_source_from_main_container(
            {
                "volumeMounts": [
                    {
                        "name": "config",
                        "mountPath": "/etc/nico/config.toml",
                        "subPath": "config.toml",
                        "readOnly": True,
                    }
                ]
            },
            "/etc/nico/config.toml",
        )
        self.assertEqual(
            mount["mountPath"],
            "/etc/fleet-intelligence/external-nico-base/config.toml",
        )
        self.assertEqual(path, mount["mountPath"])
        self.assertEqual(mount["subPath"], "config.toml")

        command_container = {
            "command": ["/usr/bin/nico", "/etc/nico/config.toml"],
            "args": ["--customer-flag"],
        }
        MODULE.replace_config_reference(
            command_container,
            "/etc/nico/config.toml",
            "/etc/fi/config.toml",
            False,
        )
        self.assertEqual(
            command_container,
            {
                "command": ["/usr/bin/nico", "/etc/fi/config.toml"],
                "args": ["--customer-flag"],
            },
        )
        with self.assertRaisesRegex(SystemExit, "reserved by the FI integration"):
            MODULE.reject_reserved_name_collisions(
                [{"name": "fleet-config", "configMap": {"name": "customer"}}],
                [{"name": "fleet-config", "configMap": {"name": "fi"}}],
                "volume",
                False,
            )


if __name__ == "__main__":
    unittest.main()
