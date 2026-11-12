# Copyright (c) 2026, NVIDIA CORPORATION & AFFILIATES. All rights reserved.
# SPDX-License-Identifier: Apache-2.0
"""Exercise host launcher settings and compare their effective defaults to Helm."""

import os
from pathlib import Path
import re
import subprocess
import tempfile
import unittest

ROOT = Path(__file__).resolve().parents[1]
PACKAGE = ROOT / "deployments/packages"
CHART = ROOT / "deployments/helm/fleet-intelligence-oob-collector"


class HostConfigTests(unittest.TestCase):
    def setUp(self):
        temporary = tempfile.TemporaryDirectory()
        self.addCleanup(temporary.cleanup)
        self.directory = Path(temporary.name)
        self.mock = self.directory / "daemon"
        self.mock.write_text('''#!/bin/sh
printf '%s\\n' "$@"
printf 'rust_log=%s\\n' "${RUST_LOG-}"
printf 'gateway_log=%s\\n' "${FLEETINT_OTEL_GATEWAY_LOG_LEVEL-}"
printf 'gateway_config=%s\\n' "${FLEETINT_GATEWAY_LOG_CONFIG-}"
printf 'token=%s\\n' "${FLEETINT_AGENT_TOKEN-}"
''')
        self.mock.chmod(0o755)
        self.launcher = self.directory / "run-service"
        self.launcher.write_text((PACKAGE / "run-service").read_text()
                                 .replace("/usr/bin/bmc-collector", str(self.mock))
                                 .replace("/usr/lib/fleetint-oob/otel-gateway", str(self.mock)))
        renderer = self.directory / "renderer"
        renderer.write_text('#!/bin/sh\nprintf "%s\\n" "$@" > "$TEST_RENDER_ARGS"\n')
        renderer.chmod(0o755)
        self.nico = self.directory / "run-nico"
        self.nico.write_text((PACKAGE / "run-nico").read_text()
                             .replace("/usr/bin/bmc-collector", str(renderer))
                             .replace("/usr/lib/fleetint-oob/nico-hardware-health", str(self.mock))
                             .replace("/run/fleetint-nico-health", str(self.directory)))
        self.defaults = dict(line.split("=", 1) for line in (PACKAGE / "config/service.conf").read_text().splitlines()
                             if line and not line.startswith("#") and "=" in line)

    def launch(self, settings=None, *, service="collector", shell="sh", nico=False):
        env = {key: value for key, value in os.environ.items()
               if not key.startswith("FLEETINT_") and key != "RUST_LOG"}
        env.update(settings or {})
        env.setdefault("FLEETINT_AGENT_TOKEN", "test-token")
        env["TEST_RENDER_ARGS"] = str(self.directory / "render-args")
        command = [shell, str(self.nico if nico else self.launcher)]
        if not nico:
            command.append(service)
        return subprocess.run(command, env=env, capture_output=True, text=True)

    def arguments(self, result):
        self.assertEqual(result.returncode, 0, result.stderr)
        return result.stdout.splitlines()[:-4]

    def helm(self, overrides=None, template="bmc-collector-deployment.yaml"):
        command = ["helm", "template", "test", str(CHART), "--show-only", f"templates/{template}",
                   "--set-json", 'inventory.targets=[{"address":"192.0.2.10","mac":"02:00:00:00:00:01"}]']
        for key, value in (overrides or {}).items():
            command.extend(["--set", f"{key}={value}"])
        result = subprocess.run(command, capture_output=True, text=True)
        self.assertEqual(result.returncode, 0, result.stderr)
        return result.stdout

    def compare_collector(self, settings, overrides=None):
        rendered = self.helm(overrides)
        block = rendered.split("          args:\n", 1)[1].split("          env:\n", 1)[0]
        expected = [arg.strip('"') for arg in re.findall(r'^\s+- (.+)$', block, re.M)]
        expected[expected.index("--config") + 1] = "/etc/fleetint-oob/config.toml"
        for shell in ("sh", "bash"):
            with self.subTest(shell=shell):
                self.assertEqual(self.arguments(self.launch(settings, shell=shell)), expected)

    def test_defaults_match_helm_with_old_and_new_service_files(self):
        self.compare_collector({})
        settings = self.defaults | {"FLEETINT_AGENT_TOKEN": "test-token"}
        self.compare_collector(settings)
        rendered = self.helm()
        for name, expected in (("RUST_LOG", self.defaults["RUST_LOG"]),
                               ("FLEETINT_NICO_LOG_LEVEL", self.defaults["FLEETINT_NICO_LOG_LEVEL"]),
                               ("FLEETINT_OTEL_GATEWAY_LOG_LEVEL", self.defaults["FLEETINT_OTEL_GATEWAY_LOG_LEVEL"])):
            self.assertRegex(rendered, rf'- name: {name}\s+value: "{expected}"')

    def test_all_collector_overrides_match_helm(self):
        self.compare_collector({
            "FLEETINT_INTERVAL_SECONDS": "60", "FLEETINT_TARGET_CONCURRENCY": "7",
            "FLEETINT_BMC_MAX_IN_FLIGHT": "3", "FLEETINT_BMC_REQUESTS_PER_SECOND": "4",
            "FLEETINT_TIMEOUT_SECONDS": "12", "FLEETINT_BMC_INSECURE_TLS": "false",
            "FLEETINT_DISABLE_LATENCY_PROBE": "true",
        }, {
            "bmcCollector.intervalSeconds": "60", "bmcCollector.targetConcurrency": "7",
            "bmcCollector.redfish.maxInFlight": "3", "bmcCollector.redfish.requestsPerSecond": "4",
            "bmcCollector.redfish.timeoutSeconds": "12", "bmcCollector.redfish.insecureTls": "false",
            "bmcCollector.latencyProbe.enabled": "false",
        })

    def test_empty_collector_settings_use_defaults(self):
        self.compare_collector({name: "" for name in (
            "FLEETINT_INTERVAL_SECONDS", "FLEETINT_TARGET_CONCURRENCY",
            "FLEETINT_BMC_MAX_IN_FLIGHT", "FLEETINT_BMC_REQUESTS_PER_SECOND",
            "FLEETINT_TIMEOUT_SECONDS", "FLEETINT_BMC_INSECURE_TLS",
            "FLEETINT_DISABLE_LATENCY_PROBE")})

    def test_zero_interval_is_preserved_and_burst_tracks_rate(self):
        args = self.arguments(self.launch({"FLEETINT_INTERVAL_SECONDS": "0", "FLEETINT_BMC_REQUESTS_PER_SECOND": "17"}))
        self.assertEqual(args[args.index("--interval-seconds") + 1], "0")
        self.assertEqual(args[args.index("--bmc-burst") + 1], "17")

    def test_invalid_settings_stop_before_starting_daemon(self):
        cases = {"FLEETINT_INTERVAL_SECONDS": ("-1", "abc"),
                 "FLEETINT_TARGET_CONCURRENCY": ("0", "-1", "abc"),
                 "FLEETINT_BMC_MAX_IN_FLIGHT": ("0", "-1", "1.2"),
                 "FLEETINT_BMC_REQUESTS_PER_SECOND": ("0", "abc"),
                 "FLEETINT_TIMEOUT_SECONDS": ("0", "-1"),
                 "FLEETINT_BMC_INSECURE_TLS": ("yes", "FALSE"),
                 "FLEETINT_DISABLE_LATENCY_PROBE": ("yes", "FALSE")}
        for shell in ("sh", "bash"):
            for name, values in cases.items():
                for value in values:
                    with self.subTest(shell=shell, setting=name, value=value):
                        result = self.launch({name: value}, shell=shell)
                        self.assertEqual(result.returncode, 2)
                        self.assertEqual(result.stdout, "")
                        self.assertIn(name, result.stderr)

    def test_token_file_precedence_and_secret_stays_out_of_arguments(self):
        token = self.directory / "token"
        token.write_text("file-token\n")
        result = self.launch({"FLEETINT_AGENT_TOKEN_FILE": str(token), "FLEETINT_AGENT_TOKEN": "legacy-token"})
        args = self.arguments(result)
        self.assertNotIn("file-token", " ".join(args))
        self.assertNotIn("legacy-token", result.stdout)
        self.assertIn("token=file-token", result.stdout)

    def test_nico_log_default_and_override_are_independent(self):
        for shell in ("sh", "bash"):
            for settings, expected in (({"RUST_LOG": "info"}, "warn"),
                                       ({"RUST_LOG": "error", "FLEETINT_NICO_LOG_LEVEL": "debug"}, "debug")):
                with self.subTest(shell=shell, settings=settings):
                    result = self.launch(settings, shell=shell, nico=True)
                    self.arguments(result)
                    self.assertIn(f"rust_log={expected}", result.stdout)
            result = self.launch({"FLEETINT_NICO_LOG_LEVEL": "invalid"}, shell=shell, nico=True)
            self.assertEqual(result.returncode, 2)
            self.assertEqual(result.stdout, "")

    def test_gateway_log_default_and_override_reach_configuration(self):
        for shell in ("sh", "bash"):
            for settings, expected in (({}, "warn"), ({"FLEETINT_OTEL_GATEWAY_LOG_LEVEL": "debug"}, "debug")):
                with self.subTest(shell=shell, settings=settings):
                    result = self.launch(settings, shell=shell, service="gateway")
                    args = self.arguments(result)
                    self.assertEqual(args, ["--config=/etc/fleetint-oob/gateway.yaml",
                                            "--config=env:FLEETINT_GATEWAY_LOG_CONFIG"])
                    self.assertIn(f"gateway_log={expected}", result.stdout)
                    self.assertIn("gateway_config=service: {telemetry: {logs: {level: " + expected + "}}}", result.stdout)
            result = self.launch({"FLEETINT_OTEL_GATEWAY_LOG_LEVEL": "trace"}, shell=shell, service="gateway")
            self.assertEqual(result.returncode, 2)
            self.assertEqual(result.stdout, "")

    def test_nico_resource_status_controls_remain_configurable(self):
        self.launch({"FLEETINT_NICO_RESOURCE_STATUS_ENABLED": "false",
                     "FLEETINT_NICO_RESOURCE_STATUS_FIRMWARE_POLL_INTERVAL_SECONDS": "90"}, nico=True)
        args = (self.directory / "render-args").read_text().splitlines()
        self.assertIn("--disable-resource-status-collection", args)
        self.assertEqual(args[args.index("--resource-status-firmware-poll-interval-seconds") + 1], "90")
