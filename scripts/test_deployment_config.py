# Copyright (c) 2026, NVIDIA CORPORATION & AFFILIATES. All rights reserved.
# SPDX-License-Identifier: Apache-2.0

"""Verify Helm configuration and Linux service settings, including shared defaults."""

from fnmatch import fnmatchcase
import json
import os
from pathlib import Path
import re
import subprocess
import sys
import tempfile
import textwrap
import tomllib
import unittest


ROOT = Path(__file__).resolve().parents[1]
CHART = ROOT / "deployments/helm/fleet-intelligence-oob-collector"
PACKAGE = ROOT / "deployments/packages"
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
printf 'token=%s\\n' "${FLEETINT_ENROLLMENT_TOKEN-}"
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
        env.setdefault("FLEETINT_ENROLLMENT_TOKEN", "test-token")
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

    def test_packaged_defaults_match_helm(self):
        settings = self.defaults | {"FLEETINT_ENROLLMENT_TOKEN": "test-token"}
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

    def test_enrollment_token_secret_is_shared_by_both_workloads(self):
        for template in ("bmc-collector-deployment.yaml", "otel-gateway-deployment.yaml"):
            with self.subTest(default_secret=template):
                rendered = self.helm(template=template)
                self.assertRegex(rendered, r'secretKeyRef:\s+'
                                 r'name: "fleet-intelligence-enrollment-token"\s+'
                                 r'key: "enrollment-token"')
        settings = {"backend.enrollmentToken.existingSecret": "collector-enrollment",
                    "backend.enrollmentToken.secretKey": "credential"}
        for template in ("bmc-collector-deployment.yaml", "otel-gateway-deployment.yaml"):
            with self.subTest(template=template):
                rendered = self.helm(settings, template)
                self.assertRegex(rendered, r'- name: FLEETINT_ENROLLMENT_TOKEN\s+valueFrom:\s+'
                                 r'secretKeyRef:\s+name: "collector-enrollment"\s+key: "credential"')
        config = self.helm(settings, "otel-gateway-configmap.yaml")
        self.assertIn("sak_token: ${env:FLEETINT_ENROLLMENT_TOKEN}", config)

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
        result = self.launch({"FLEETINT_ENROLLMENT_TOKEN_FILE": str(token), "FLEETINT_ENROLLMENT_TOKEN": "environment-token"})
        args = self.arguments(result)
        self.assertNotIn("file-token", " ".join(args))
        self.assertNotIn("environment-token", result.stdout)
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


class DsxPublicationTests(unittest.TestCase):
    def job(self, name):
        workflow = (ROOT / ".github/workflows/release.yml").read_text()
        match = re.search(rf"^  {re.escape(name)}:\n(.*?)(?=^  [\w-]+:|\Z)",
                          workflow, re.MULTILINE | re.DOTALL)
        self.assertIsNotNone(match, f"Missing release job: {name}")
        return match[1]

    def setUp(self):
        directory = tempfile.TemporaryDirectory()
        self.addCleanup(directory.cleanup)
        self.directory = Path(directory.name)
        self.dist = self.directory / "dist"
        self.dist.mkdir()
        self.existing = self.directory / "registry-chart"
        self.log = self.directory / "calls.jsonl"
        fixture = "#!" + sys.executable + "\n" + r"""
import hashlib
import json
import os
from pathlib import Path
import shutil
import sys

tool = Path(sys.argv[0]).name
args = sys.argv[1:]
with open(os.environ["FIXTURE_LOG"], "a") as log:
    log.write(json.dumps({"tool": tool, "args": args}) + "\n")


def fail(message):
    print(message, file=sys.stderr)
    sys.exit(1)


def auth(option, expected):
    value = json.loads(Path(args[args.index(option) + 1]).read_text())
    if value.get("role", "anonymous") not in expected:
        fail("wrong credential used for registry")


manifest = b'{"schemaVersion":2,"manifests":[{"platform":{"os":"linux","architecture":"amd64"}},{"platform":{"os":"linux","architecture":"arm64"}}]}'
if tool == "helm":
    assert args[0] == "template"
    print('    image: "nvcr.io/nvidia/fleet-intelligence/otel-gateway:4.5.6"')
elif tool == "skopeo":
    if args[0] == "login":
        Path(args[args.index("--authfile") + 1]).write_text(json.dumps({"role": sys.stdin.read()}))
    elif args[0] == "inspect":
        reference = args[-1]
        destination = reference.startswith("docker://nvcr.io/example-org/artifacts/")
        auth("--authfile", {"destination-key"} if destination else {"source-key", "anonymous"})
        if destination:
            if os.environ.get("IMAGE_MODE") == "denied":
                fail("denied: insufficient repository permissions")
            stored = Path(os.environ["RUNNER_TEMP"]) / hashlib.sha256(reference.encode()).hexdigest()
            if os.environ.get("IMAGE_MODE") == "conflict":
                sys.stdout.buffer.write(b"different image manifest")
            elif stored.exists():
                sys.stdout.buffer.write(stored.read_bytes())
            else:
                fail('time="2026-11-17T22:01:05Z" level=fatal msg="reading manifest: manifest unknown"')
        else:
            sys.stdout.buffer.write(manifest)
    elif args[0] == "copy":
        assert "--all" in args and "--preserve-digests" in args
        auth("--src-authfile", {"source-key", "anonymous"})
        auth("--dest-authfile", {"destination-key"})
        assert args[-2].endswith("@sha256:" + hashlib.sha256(manifest).hexdigest())
        assert args[-1].startswith("docker://nvcr.io/example-org/artifacts/")
        stored = Path(os.environ["RUNNER_TEMP"]) / hashlib.sha256(args[-1].encode()).hexdigest()
        stored.write_bytes(manifest)
    else:
        fail("unknown skopeo operation")
elif tool == "ngc":
    assert args[:2] == ["registry", "chart"]
    assert os.environ["NGC_CLI_API_KEY"] == "destination-key"
    existing = Path(os.environ["EXISTING_CHART"])
    mode = os.environ["MODE"]
    if args[2] in ("info", "create"):
        pass
    elif args[2] == "push":
        if existing.exists() or mode in ("missing", "auth"):
            fail("chart push failed")
        source = Path(os.environ["CHART_BASENAME"])
        existing.write_bytes(b"corrupt archive" if mode == "corrupt" else source.read_bytes())
        if mode == "interrupted":
            fail("lost upload response")
    elif args[2] == "pull":
        if mode == "auth" or not existing.exists():
            fail("chart pull failed")
        shutil.copyfile(existing, Path(args[args.index("--dest") + 1]) / os.environ["CHART_BASENAME"])
    else:
        fail("unknown ngc operation")
"""
        for tool in ("ngc", "skopeo", "helm"):
            executable = self.directory / tool
            executable.write_text(fixture)
            executable.chmod(0o755)

    def publish(self, mode="normal", *, prefix="", anonymous=False, image_mode="normal",
                version="1.2.3-rc.1"):
        basename = f"{prefix}fleet-intelligence-oob-collector-{version}.tgz"
        chart = self.dist / basename
        chart.write_bytes(b"published chart archive")
        env = dict(os.environ, PATH=str(self.directory) + os.pathsep + os.environ["PATH"],
                   RUNNER_TEMP=str(self.directory), VERSION=version, MODE=mode,
                   DSX_NGC_ORG_TEAM="example-org/artifacts", DSX_KEY="destination-key",
                   SOURCE_KEY="" if anonymous else "source-key", DSX_ARTIFACT_PREFIX=prefix,
                   IMAGE="nvcr.io/source/published/oob-collector",
                   NICO_IMAGE="nvcr.io/source/published/nico-hardware-health",
                   EXISTING_CHART=str(self.existing), CHART_BASENAME=basename,
                   FIXTURE_LOG=str(self.log), IMAGE_MODE=image_mode)
        return subprocess.run(["bash", str(ROOT / "scripts/publish-dsx.sh"), str(chart)],
                              cwd=self.directory, env=env, capture_output=True, text=True, timeout=10)

    def calls(self, tool, operation):
        return [call["args"] for call in map(json.loads, self.log.read_text().splitlines())
                if call["tool"] == tool and operation in call["args"]]

    def test_dsx_retry_is_separate_from_public_publication(self):
        job = self.job("publish-dsx")
        self.assertRegex(job, r"needs:\s*\[validate,\s*publish-chart\]")
        self.assertIn("environment: ngc-publish", job)
        self.assertIn("Download the published Helm chart", job)
        self.assertIn("scripts/publish-dsx.sh", job)
        self.assertNotIn("scripts/publish-dsx.sh", self.job("publish-chart"))

    def test_private_dispatch_skips_normal_release_and_preserves_environment(self):
        validation = self.job("validate")
        self.assertRegex(validation, r"github.ref_type == 'tag'.*!inputs.dsx_test_only")
        self.assertIn("!startsWith(github.ref_name, 'v-dsx-test-')", validation)
        smoke = self.job("test-dsx-publication")
        self.assertIn("environment: ngc-publish", smoke)
        self.assertIn("github.event_name == 'workflow_dispatch'", smoke)
        self.assertIn("github.ref_type == 'tag'", smoke)
        self.assertIn("inputs.dsx_test_only", smoke)
        self.assertIn("startsWith(github.ref_name, 'v-dsx-test-')", smoke)
        self.assertIn("!contains(github.ref_name, '.')", smoke)
        self.assertNotIn("needs:", smoke)
        self.assertIn("scripts/publish-dsx.sh", smoke)

        workflow = (ROOT / ".github/workflows/release.yml").read_text()
        tag_filters = workflow.split("  push:\n", 1)[1].split("  workflow_dispatch:", 1)[0]
        tag_patterns = re.findall(r"- ['\"]([^'\"]+)['\"]", tag_filters)
        self.assertTrue(tag_patterns)
        for tag in ("v-dsx-test-pr114", "v-dsx-test-pr114-20261117"):
            self.assertFalse(any(fnmatchcase(tag, pattern) for pattern in tag_patterns), tag)
        self.assertTrue(any(fnmatchcase("v1.2.3-rc.1", pattern) for pattern in tag_patterns))

        def depends_on_validation(name):
            if name == "validate":
                return True
            needs = re.search(r"^    needs:\s*(.+)$", self.job(name), re.MULTILINE)
            self.assertIsNotNone(needs, f"{name} must depend on release validation")
            return any(depends_on_validation(dependency)
                       for dependency in re.findall(r"[\w-]+", needs[1]))

        for name in ("build-image", "publish-image", "build-nico-image", "publish-nico-image",
                     "publish-chart", "publish-dsx", "reserve-package-release", "build-packages",
                     "sign-packages", "upload-package-assets"):
            self.assertTrue(depends_on_validation(name), name)
            self.assertNotRegex(self.job(name), r"^    if:.*always\(")

    def test_invalid_release_or_test_prefix_fails_before_authentication(self):
        for settings in ({"version": "latest"}, {"prefix": "production-"},
                         {"prefix": "dsx-test-123-"}):
            with self.subTest(settings=settings):
                self.assertNotEqual(self.publish(**settings).returncode, 0)
                self.assertFalse(self.log.exists())

    def test_new_duplicate_and_lost_success_uploads_accept_identical_bytes(self):
        for mode in ("normal", "duplicate", "interrupted"):
            with self.subTest(mode=mode):
                self.existing.unlink(missing_ok=True)
                if mode == "duplicate":
                    self.existing.write_bytes(b"published chart archive")
                result = self.publish(mode)
                self.assertEqual(result.returncode, 0, result.stderr)
                self.assertEqual(self.existing.read_bytes(), b"published chart archive")

    def test_conflicting_existing_chart_is_rejected_without_replacement(self):
        self.existing.write_bytes(b"different chart archive")
        self.assertNotEqual(self.publish("duplicate").returncode, 0)
        self.assertEqual(self.existing.read_bytes(), b"different chart archive")

    def test_failed_push_requires_successful_pull_before_accepting_existing_chart(self):
        for mode in ("missing", "auth"):
            with self.subTest(mode=mode):
                self.existing.unlink(missing_ok=True)
                if mode == "auth":
                    self.existing.write_bytes(b"published chart archive")
                self.assertNotEqual(self.publish(mode).returncode, 0)

    def test_successful_chart_upload_is_read_back_and_checked(self):
        self.assertNotEqual(self.publish("corrupt").returncode, 0)
        self.assertEqual(len(self.calls("ngc", "pull")), 1)

    def test_private_test_names_and_anonymous_reads_stay_separate(self):
        prefix = "dsx-test-123-1-"
        result = self.publish(prefix=prefix, anonymous=True)
        self.assertEqual(result.returncode, 0, result.stderr)
        copies = self.calls("skopeo", "copy")
        self.assertEqual(len(copies), 3)
        expected = ["fleet-intelligence-oob-collector:1.2.3-rc.1",
                    "fleet-intelligence-nico-hardware-health:1.2.3-rc.1",
                    "fleet-intelligence-otel-gateway:4.5.6"]
        self.assertEqual([copy[-1] for copy in copies],
                         ["docker://nvcr.io/example-org/artifacts/" + prefix + name for name in expected])
        self.assertEqual(len(self.calls("skopeo", "login")), 1)
        for copy in copies:
            self.assertNotEqual(copy[copy.index("--src-authfile") + 1],
                                copy[copy.index("--dest-authfile") + 1])
        pushes = self.calls("ngc", "push")
        self.assertEqual(len(pushes), 1)
        self.assertIn(f"example-org/artifacts/{prefix}fleet-intelligence-oob-collector:1.2.3-rc.1", pushes[0])
        # Retrying the exact test targets accepts existing bytes without copying images again.
        self.assertEqual(self.publish(prefix=prefix, anonymous=True).returncode, 0)
        self.assertEqual(len(self.calls("skopeo", "copy")), 3)

    def test_conflicting_image_digest_stops_before_any_write(self):
        self.assertNotEqual(self.publish(image_mode="conflict").returncode, 0)
        self.assertEqual(self.calls("skopeo", "copy"), [])
        self.assertEqual(self.calls("ngc", "push"), [])

    def test_denied_image_read_reports_diagnostic_without_attempting_copy(self):
        result = self.publish(image_mode="denied")
        self.assertNotEqual(result.returncode, 0)
        self.assertIn("Registry inspection: denied: insufficient repository permissions", result.stderr)
        self.assertEqual(self.calls("skopeo", "copy"), [])
        self.assertEqual(self.calls("ngc", "push"), [])


if __name__ == "__main__":
    unittest.main()
