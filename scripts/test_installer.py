# Copyright (c) 2026, NVIDIA CORPORATION & AFFILIATES. All rights reserved.
# SPDX-License-Identifier: Apache-2.0

import fcntl
import json
import os
from pathlib import Path
import pty
import re
import select
import shutil
import signal
import struct
import subprocess
import sys
import tempfile
import termios
import time
import unittest


ROOT = Path(__file__).resolve().parents[1]
SCRIPT = (ROOT / "install.sh").read_text()
CHART_LOOKUP = ["helm", "show", "chart", "fleet-intelligence-oob-collector",
                "--repo", "https://helm.ngc.nvidia.com/nvidia/fleet-intelligence"]


def terminal_screen(text, columns, rows=24):
    cells = [[(" ", False) for _ in range(columns)] for _ in range(rows)]
    row = column = 0
    shaded = False
    saved = (0, 0, False)
    for token in re.findall(r"\x1b\[[0-?]*[ -/]*[@-~]|\x1b[78]|[^\x1b]", text):
        if token == "\x1b7":
            saved = (row, column, shaded)
        elif token == "\x1b8":
            row, column, shaded = saved
        elif token.startswith("\x1b["):
            command, raw = token[-1], token[2:-1]
            if raw.startswith("?"):
                continue
            values = [int(n or 0) for n in raw.split(";")]
            count = values[0] or 1
            if command == "H":
                row, column = count - 1, (values[1] if len(values) > 1 else 1) - 1
            elif command == "G":
                column = count - 1
            elif command == "A":
                row = max(0, row - count)
            elif command == "B":
                row += count
            elif command in ("J", "K"):
                for r in range(row, rows if command == "J" else min(row + 1, rows)):
                    for c in range(column if r == row else 0, columns):
                        cells[r][c] = (" ", shaded)
            elif command == "m":
                i = 0
                while i < len(values):
                    if values[i] == 0:
                        shaded = False
                    elif values[i] in (38, 48) and values[i + 1:i + 2] == [2]:
                        if values[i] == 48:
                            shaded = values[i + 2:i + 5] == [24, 32, 39]
                        i += 4
                    i += 1
        elif token == "\r":
            column = 0
        elif token == "\n":
            row += 1
        elif token >= " ":
            if column >= columns:
                row, column = row + 1, 0
            if 0 <= row < rows and 0 <= column < columns:
                cells[row][column] = (token, shaded)
            column += 1
    return cells


class InstallerTests(unittest.TestCase):
    def setUp(self):
        self.temp = tempfile.TemporaryDirectory()
        self.addCleanup(self.temp.cleanup)
        self.directory = Path(self.temp.name)

    def validate(self, function, value):
        body = SCRIPT.split("  ui_mode=auto\n", 1)[0]
        code = body + '\n  "$1" "$2"\n  printf "%s" "$REPLY"\n}\nfleetint_install "$@"\n'
        return subprocess.run(["bash", "-c", code, "validator", function, value], text=True, capture_output=True)

    def test_validators_match_supported_target_inputs(self):
        for value in ("192.0.2.1", "0.0.0.0", "255.255.255.255", "2001:db8::1", "::", "::1", "::ffff:192.0.2.1", "1:2:3:4:5:6:7:8", "1:2:3:4:5:6:192.0.2.1"):
            with self.subTest(value=value):
                self.assertEqual(self.validate("valid_ip", value).returncode, 0)
        for value in ("https://192.0.2.1", "192.0.2.1:443", "fe80::1%eth0", "example.com", "256.0.0.1", "01.2.3.4", "1.2.3.4.5", "1:::2", "1::2::3", "1:2:3:4:5:6:7", "1:2:3:4:5:6:7:8::", ":1:2:3:4:5:6:7:8", "1:2:3:4:5:6:7:8:", "::ffff:999.0.0.1"):
            with self.subTest(value=value):
                self.assertNotEqual(self.validate("valid_ip", value).returncode, 0)
        for value in ("00:11:22:33:aa:bb", "00-11-22-33-AA-BB", "00112233aabb"):
            self.assertEqual(self.validate("valid_mac", value).returncode, 0)
            self.assertEqual(self.validate("normalize_mac", value).stdout, "00:11:22:33:AA:BB")
        self.assertNotEqual(self.validate("valid_mac", "00:11-22:33:44:55").returncode, 0)
        for value in ("http://example.com", "https://u:p@example.com", "https://example.com/path", "https://example.com?token=x", "https://example.com:bad", "https://example.com:0", "https://example.com:65536"):
            self.assertNotEqual(self.validate("valid_backend", value).returncode, 0)
        for value in ("https://data.fleet-intelligence.nvidia.com", "https://[2001:db8::1]:443", "https://example.com:8443/"):
            self.assertEqual(self.validate("valid_backend", value).returncode, 0)
        for value in ("0", "65536", "-1", "1+2", "$(touch SENTINEL)"):
            self.assertNotEqual(self.validate("valid_port", value).returncode, 0)
        for value in ("1", "443", "65535", "00000443"):
            self.assertEqual(self.validate("valid_port", value).returncode, 0)

    def test_questionnaire_handles_multiple_targets_and_retries_invalid_answers(self):
        answers = ["test-context", "", "", "1.2.3", "", "zone", "group", "no",
                   "compute", "not-an-ip", "192.0.2.10", "001122334455", "", "node-01", "bmc", "", "", "rack", "rack-12", "",
                   "yes", "nvswitch", "2001:db8::1", "00-11-22-33-44-55", "001122334456", "8443",
                   "switch-01", "switch", "other-zone", "other-group", "", "no",
                   "existing", "", "", "existing", "", "", "pull-secret", "yes"]
        code, output, _, saved = self.run_script(answers=answers)
        self.assertEqual(code, 0, output)
        values = json.loads(saved.read_text())
        targets = values["inventory"]["targets"]
        self.assertEqual(len(targets), 2)
        self.assertEqual(targets[1]["mac"], "00:11:22:33:44:56")
        self.assertEqual(targets[1]["computeZone"], "other-zone")
        self.assertEqual(targets[1]["port"], 8443)
        self.assertEqual(targets[0]["labels"], {"rack": "rack-12"})
        self.assertEqual(values["imagePullSecrets"], [{"name": "pull-secret"}])

    def run_script(self, *, mode="existing", fail="", confirm="yes", real_render=False, answers=None,
                   no_python=False, display_name="$(touch SENTINEL)", ui="plain", arrows=False, cancel_at=None,
                   no_color=False, ascii_only=False, helm_version="v3.19.0", columns=80, rows=24,
                   color_mode="truecolor", echo=False, terminal=None, input_delay=0, input_signal=None,
                   missing_stty=False, glyphs=None, chart_token="enrollmentToken", existing_nico="no",
                   version_answer="1.2.3", chart_metadata="appVersion: 9.9.9\nversion: 1.4.2", install_env=None,
                   result_keys=("\n",)):
        mock = self.directory / "mock.py"
        mock.write_text("#!" + sys.executable + "\n" + '''import json, os, pathlib, signal, subprocess, sys, time
name, args = pathlib.Path(sys.argv[0]).name, sys.argv[1:]
with open(os.environ['CALL_LOG'], 'a') as log:
    log.write(json.dumps([name, *args]) + '\\n')
mode, failure = os.environ['TEST_MODE'], os.environ['FAIL_AT']
if name == 'helm':
    if args == ['version', '--short']:
        print(os.environ['TEST_HELM_VERSION'])
    if args[:2] == ['show', 'chart']:
        if failure == 'chart-lookup':
            sys.exit(1)
        print(os.environ['CHART_METADATA'])
    if args[:2] == ['show', 'values']:
        if failure == 'show-values':
            sys.exit(1)
        print(os.environ['CHART_DEFAULTS'])
    if '--values' in args:
        path = pathlib.Path(args[args.index('--values') + 1])
        pathlib.Path(os.environ['SAVED_VALUES']).write_text(path.read_text())
    if failure == 'template' and args[0] == 'template':
        sys.exit(1)
    if args[0] == 'template' and os.environ['REAL_RENDER']:
        sys.exit(subprocess.run([os.environ['REAL_HELM'], 'template', 'test', os.environ['CHART'], '--values', str(path)]).returncode)
    if failure == 'upgrade' and args[0] == 'upgrade':
        sys.exit(1)
    if failure == 'terminate' and args[0] == 'upgrade':
        os.kill(os.getppid(), signal.SIGTERM)
        time.sleep(5)
elif 'secret' in args and 'get' in args:
    if failure == 'secret-read':
        sys.exit(1)
    if args[args.index('-o') + 1] == 'name':
        if mode == 'existing':
            print('secret/example')
    else:
        print('0' if failure == 'empty-key' else '24')
elif 'create' in args and 'namespace' in args:
    print('namespace fixture')
elif 'apply' in args:
    sys.stdin.read()
elif 'wait' in args and failure == 'namespace-wait':
    print('error: namespace never became Active', file=sys.stderr)
    sys.exit(1)
elif 'get' in args and 'deployment' in args:
    selector = args[args.index('--selector') + 1]
    if ',app.kubernetes.io/component=nico-hardware-health' in selector:
        if failure == 'missing-nico':
            sys.exit(0)
        deployment = 'fixture-nico'
    elif ',app.kubernetes.io/component=otel-gateway' in selector:
        deployment = 'fixture-gateway'
    else:
        deployment = 'fixture-collector'
    print('deployment.apps/' + deployment)
elif 'rollout' in args and failure == 'rollout' and 'deployment.apps/fixture-nico' in args:
    print('error: NICo Deployment exceeded its progress deadline', file=sys.stderr)
    sys.exit(1)
elif 'get' in args and 'pods,replicasets' in args:
    print('release-pod-uid')
elif 'get' in args and 'pods' in args:
    print('fixture-nico  0/1  Init:CrashLoopBackOff' if failure == 'rollout' else 'Fixture pods')
elif 'get' in args and 'events' in args:
    print('Warning FailedMount: the credentials key could not be mounted')
elif 'logs' in args and '--tail=500' in args:
    if failure == 'logs-forbidden':
        print('Error from server (Forbidden): cannot get resource "pods/log"', file=sys.stderr)
        sys.exit(1)
    if failure == 'inventory-timeout':
        print('2026-01-01T00:00:00Z INFO fleet_intelligence_oob: inventory backend sync completed accepted_upload_count=0')
        print('2026-01-01T00:00:00Z ERROR other: INFO fleet_intelligence_oob: first inventory upload accepted in this cycle')
        print('2026-01-01T00:00:00Z WARN fleet_intelligence_oob: BMC login failed')
    elif failure == 'legacy-inventory':
        print('2026-01-01T00:00:00Z INFO fleet_intelligence_oob: inventory mini-batch export completed exportable_count=1')
    else:
        polls = sum('--tail=500' in json.loads(line) for line in pathlib.Path(os.environ['CALL_LOG']).read_text().splitlines())
        if failure == 'inventory-retry' and polls == 1:
            print('2026-01-01T00:00:00Z INFO fleet_intelligence_oob: backend enrollment succeeded')
        else:
            print('2026-01-01T00:00:00Z INFO fleet_intelligence_oob: first inventory upload accepted in this cycle accepted_upload_count=1')
elif 'logs' in args:
    container = args[args.index('--container') + 1]
    if container == 'generate-nico-config':
        print('Error: invalid NICo configuration')
    if container == 'otel-gateway' and failure:
        print('2026-01-01T00:00:00Z\\terror\\tOTLP export failed: connection timeout')
    if container == 'collector' and failure:
        print('2026-01-01T00:00:00Z WARN fleet_intelligence_oob: BMC login failed')
        if failure == 'rollout':
            print('\\x1b]0;unsafe-title\\x07Scoped collector log')
''')
        mock.chmod(0o700)
        for name in ("helm", "kubectl"):
            (self.directory / name).symlink_to(mock)
        sleep = self.directory / "sleep"
        sleep.write_text("#!/bin/sh\nexit 0\n")
        sleep.chmod(0o700)
        log, saved = self.directory / "calls.jsonl", self.directory / "saved-values.json"
        private_file = self.directory / "private input $(touch SENTINEL)"
        private_file.write_text("fixture-secret-never-print")
        if fail == "missing-file":
            private_file.unlink()
        if answers is None:
            answers = ["test-context", "", "", version_answer, "", "zone", "group", existing_nico,
                       "compute", "192.0.2.10", "001122334455", "", display_name, "bmc", "", "", "", "no"]
            for _ in range(2):
                answers += [mode, "", ""] + ([str(private_file)] if mode == "file" else [])
            answers += ["", confirm]
        if no_python:
            for name in ("mktemp", "chmod", "rm", "stty"):
                if name == "stty" and missing_stty:
                    continue
                (self.directory / name).symlink_to(shutil.which(name))
        env = {key: value for key, value in os.environ.items() if not key.startswith("FLEETINT_INSTALL_")}
        env.update(PATH=str(self.directory) + os.pathsep + os.environ["PATH"],
                   CALL_LOG=str(log), TEST_MODE=mode, FAIL_AT=fail, SAVED_VALUES=str(saved), TEST_HELM_VERSION=helm_version,
                   CHART_METADATA=chart_metadata,
                   CHART_DEFAULTS="backend:\n" + "".join("  " + field + ":\n    existingSecret: fixture\n" for field in (["agentToken", "enrollmentToken"] if chart_token == "both" else [chart_token] if chart_token else [])),
                   TMPDIR=str(self.directory), REAL_RENDER="yes" if real_render else "",
                   REAL_HELM=shutil.which("helm") or "", CHART=str(ROOT / "deployments/helm/fleet-intelligence-oob-collector"))
        env.update(install_env or {})
        if no_python:
            env["PATH"] = str(self.directory)
        env["TERM"] = "dumb" if ui in ("fallback", "unsupported") else "xterm-256color"
        if terminal is not None:
            env["TERM"] = terminal
        env["COLORTERM"] = color_mode
        if color_mode == "ansi":
            env["TERM"] = "xterm"
        env["LC_ALL"] = "C" if ascii_only else "en_US.UTF-8"
        env["NO_COLOR"] = "1" if no_color else ""
        master, slave = pty.openpty()
        fcntl.ioctl(slave, termios.TIOCSWINSZ, struct.pack("HHHH", rows, columns, 0, 0))
        settings = termios.tcgetattr(slave)
        if echo:
            settings[3] |= termios.ECHO
        else:
            settings[3] &= ~termios.ECHO
        termios.tcsetattr(slave, termios.TCSANOW, settings)
        settings = termios.tcgetattr(slave)
        def terminal_session():
            os.setsid()
            fcntl.ioctl(slave, termios.TIOCSCTTY, 0)
        options = [] if ui in ("auto", "fallback") else ["--tui" if ui == "unsupported" else "--" + ui]
        if glyphs is not None:
            options.append("--" + glyphs)
        process = subprocess.Popen([shutil.which("bash"), "-s", "--", *options], stdin=subprocess.PIPE, stdout=slave, stderr=slave,
                                   cwd=self.directory, env=env, pass_fds=(slave,), preexec_fn=terminal_session)
        os.close(slave)
        output, pending, index = b"", b"", 0
        result_index = 0
        menu_confirmation = False
        cancelled = False
        try:
            # The script arrives on stdin while answers arrive independently through /dev/tty.
            process.stdin.write(SCRIPT.encode())
            process.stdin.close()
            deadline = time.monotonic() + 20
            while time.monotonic() < deadline:
                if not select.select([master], [], [], 0.1)[0]:
                    if process.poll() is not None:
                        break
                    continue
                try:
                    chunk = os.read(master, 65536)
                except OSError:
                    break
                if not chunk:
                    break
                output += chunk
                pending += chunk
                screen = re.sub(rb"\x1b\[[0-?]*[ -/]*[@-~]", b"", pending)
                if cancelled:
                    continue
                if ui in ("tui", "auto"):
                    if screen.endswith(b"1: Checks. 2: Logs. Enter: close.\r\n"):
                        self.assertLess(result_index, len(result_keys), "verification panel asked for another key")
                        os.write(master, result_keys[result_index].encode())
                        result_index += 1
                        pending = b""
                    elif screen.endswith(b"Press Enter to try again."):
                        os.write(master, b"\n")
                        pending = b""
                    elif b"Page " in screen and screen.endswith(b"Esc: cancel.\r\n"):
                        os.write(master, b"\n")
                        pending = b""
                    elif screen.endswith(b"Enter or number: select. Esc: cancel.\r\n") and index < len(answers):
                        if cancel_at == index:
                            os.write(master, b"\x1b")
                            cancelled = True
                        elif menu_confirmation:
                            os.write(master, b"\n")
                            menu_confirmation = False
                            index += 1
                        elif answers[index] == "":
                            os.write(master, b"\n")
                            index += 1
                        else:
                            if b"Kind: compute" in screen or b"Choose the hardware type" in screen:
                                choices = ["unknown", "compute", "nvswitch", "power_shelf"]
                            elif b"Source: existing or file" in screen or b"Where are your credentials stored?" in screen:
                                choices = ["existing", "file"]
                            else:
                                choices = ["no", "yes"]
                            selected = choices.index(answers[index])
                            if arrows and selected:
                                self.assertEqual(selected, 1)
                                os.write(master, b"\x1b[B")
                                menu_confirmation = True
                            else:
                                os.write(master, b"\n" if arrows else str(selected + 1).encode())
                                index += 1
                        pending = b""
                    elif b"\x1b[4A" in pending and b"\x1b8\x1b[?25h" in pending and index < len(answers):
                        if index == 0 and input_delay:
                            time.sleep(input_delay)
                        if index == 0 and input_signal:
                            os.kill(process.pid, input_signal)
                            cancelled = True
                        elif cancel_at == index:
                            os.write(master, b"\x03")
                            cancelled = True
                        else:
                            os.write(master, (answers[index] + "\n").encode())
                            index += 1
                        pending = b""
                elif pending.endswith(b": ") and index < len(answers):
                    os.write(master, (answers[index] + "\n").encode())
                    index += 1
                    pending = b""
            try:
                code = process.wait(timeout=2)
            except subprocess.TimeoutExpired:
                self.fail(f"Installer stopped responding at answer {index}, menu confirm {menu_confirmation}, pending {pending!r}: {output[-4000:]!r}")
            self.assertEqual(termios.tcgetattr(master), settings, "terminal settings must be restored")
        finally:
            if process.poll() is None:
                os.killpg(process.pid, signal.SIGKILL)
                process.wait()
            os.close(master)
        output = output.decode(errors="replace")
        self.assertNotIn("fixture-secret-never-print", output)
        self.assertFalse((self.directory / "SENTINEL").exists())
        self.assertFalse(list(self.directory.glob("fleetint-install-*")))
        calls = [json.loads(line) for line in log.read_text().splitlines()]
        return code, output, calls, saved

    def test_new_deployment_uses_bundled_health_defaults(self):
        code, output, _, saved = self.run_script(existing_nico="", real_render=bool(shutil.which("helm")))
        self.assertEqual(code, 0, output)
        values = json.loads(saved.read_text())
        self.assertNotIn("nicoHardwareHealth", values)
        self.assertNotIn("otelGateway", values)
        self.assertIn("first inventory upload accepted in this cycle", output)

    def test_enter_accepts_and_pins_the_latest_chart_in_plain_and_tui_modes(self):
        for ui in ("plain", "tui"):
            with self.subTest(ui=ui), tempfile.TemporaryDirectory() as directory:
                self.directory = Path(directory)
                code, output, calls, _ = self.run_script(ui=ui, version_answer="")
                self.assertEqual(code, 0, output)
                self.assertIn("NGC chart version [1.4.2]" if ui == "plain" else "Default: 1.4.2", output)
                self.assertIn("Chart version: 1.4.2", output)
                self.assertEqual(calls[1], CHART_LOOKUP)
                for command in (["show", "values"], ["template"], ["upgrade", "--install"]):
                    call = next(c for c in calls if c[:len(command) + 1] == ["helm", *command])
                    self.assertEqual(call[call.index("--version") + 1], "1.4.2")

    def test_unavailable_default_leaves_an_exact_version_required(self):
        cases = (("chart-lookup", ""), ("", "appVersion: 9.9.9"),
                 ("", "version: 2.0.0-rc.1"))
        for fail, metadata in cases:
            with self.subTest(fail=fail, metadata=metadata), tempfile.TemporaryDirectory() as directory:
                self.directory = Path(directory)
                answers = ["test-context", "", "", "", "1.2.3", "", "zone", "group", "no",
                           "compute", "192.0.2.10", "001122334455", "", "node", "bmc", "", "", "", "no",
                           "existing", "", "", "existing", "", "", "", "yes"]
                code, output, calls, _ = self.run_script(fail=fail, chart_metadata=metadata, answers=answers)
                self.assertEqual(code, 0, output)
                self.assertIn("Latest release lookup failed", output)
                upgrade = next(c for c in calls if c[:3] == ["helm", "upgrade", "--install"])
                self.assertEqual(upgrade[upgrade.index("--version") + 1], "1.2.3")

    def test_exported_defaults_are_labeled_and_used_in_plain_and_tui_modes(self):
        for ui in ("plain", "tui"):
            with self.subTest(ui=ui), tempfile.TemporaryDirectory() as directory:
                self.directory = Path(directory)
                credentials = self.directory / "BMC credentials.json"
                enrollment = self.directory / "enrollment-token"
                for path in (credentials, enrollment):
                    path.write_text("fixture-secret-never-print")
                defaults = {
                    "KUBE_CONTEXT": "env-context", "NAMESPACE": "env-ns", "RELEASE": "env-release",
                    "CHART_VERSION": "2.3.4", "BACKEND_URL": "https://backend.example",
                    "COMPUTE_ZONE": "env-zone", "NODE_GROUP": "env-group",
                    "BMC_KIND": "nvswitch", "BMC_IP": "192.0.2.1", "BMC_MAC": "02-00-00-00-00-01",
                    "BMC_PORT": "08443", "BMC_NAME": "env-device", "BMC_CREDENTIALS_PROFILE": "switch",
                    "BMC_COMPUTE_ZONE": "target-zone", "BMC_NODE_GROUP": "target-group",
                    "BMC_CREDENTIALS_FILE": str(credentials), "BMC_CREDENTIALS_SECRET_NAME": "env-bmc",
                    "BMC_CREDENTIALS_SECRET_KEY": "bmc.json", "ENROLLMENT_TOKEN_FILE": str(enrollment),
                    "ENROLLMENT_TOKEN_SECRET_NAME": "env-enrollment", "ENROLLMENT_TOKEN_SECRET_KEY": "token",
                    "IMAGE_PULL_SECRET": "env-registry",
                }
                exported = {f"FLEETINT_INSTALL_{key}": value for key, value in defaults.items()}
                # Accept the environment defaults but still answer the final confirmation.
                answers = [""] * 7 + ["no"] + [""] * 9 + ["no"] + [""] * 9 + ["yes"]
                code, output, calls, saved = self.run_script(
                    ui=ui, mode="file", answers=answers, install_env=exported,
                )
                self.assertEqual(code, 0, output)
                for key in exported:
                    self.assertIn(f"From environment: {key}", output)
                self.assertNotIn(CHART_LOOKUP, calls)
                upgrade = next(c for c in calls if c[:3] == ["helm", "upgrade", "--install"])
                self.assertEqual(upgrade[3], "env-release")
                for flag, value in (("--kube-context", "env-context"), ("--namespace", "env-ns"), ("--version", "2.3.4")):
                    self.assertEqual(upgrade[upgrade.index(flag) + 1], value)
                values = json.loads(saved.read_text())
                self.assertEqual(values["inventory"], {
                    "computeZone": "env-zone", "nodeGroup": "env-group", "targets": [{
                        "address": "192.0.2.1", "mac": "02:00:00:00:00:01", "kind": "nvswitch",
                        "port": 8443, "name": "env-device", "credentialsProfile": "switch",
                        "computeZone": "target-zone", "nodeGroup": "target-group",
                    }],
                })
                self.assertEqual(values["credentials"], {"existingSecret": "env-bmc", "secretKey": "bmc.json"})
                self.assertEqual(values["backend"], {"url": "https://backend.example", "enrollmentToken": {
                    "existingSecret": "env-enrollment", "secretKey": "token",
                }})
                self.assertEqual(values["imagePullSecrets"], [{"name": "env-registry"}])
                secret_creates = [c for c in calls if c[0] == "kubectl" and "create" in c and "secret" in c]
                self.assertEqual(len(secret_creates), 2)
                self.assertIn(f"--from-file=bmc.json={credentials}", secret_creates[0])
                self.assertIn(f"--from-file=token={enrollment}", secret_creates[1])

    def test_answers_override_defaults_and_bmc_environment_only_applies_to_first_target(self):
        exported = {
            "FLEETINT_INSTALL_NAMESPACE": "env-ns",
            "FLEETINT_INSTALL_CHART_VERSION": "2.3.4",
            "FLEETINT_INSTALL_BMC_KIND": "nvswitch",
            "FLEETINT_INSTALL_BMC_IP": "192.0.2.1",
            "FLEETINT_INSTALL_BMC_MAC": "02:00:00:00:00:01",
            "FLEETINT_INSTALL_BMC_PORT": "8443",
        }
        answers = ["test-context", "manual-ns", "", "1.2.3", "", "", "", "no",
                   "compute", "", "", "", "", "bmc", "", "", "", "yes",
                   "", "192.0.2.2", "02:00:00:00:00:02", "", "", "bmc", "", "", "", "",
                   "existing", "", "", "existing", "", "", "", "yes"]
        code, output, calls, saved = self.run_script(ui="tui", install_env=exported, answers=answers)
        self.assertEqual(code, 0, output)
        upgrade = next(c for c in calls if c[:3] == ["helm", "upgrade", "--install"])
        self.assertEqual(upgrade[upgrade.index("--namespace") + 1], "manual-ns")
        self.assertEqual(upgrade[upgrade.index("--version") + 1], "1.2.3")
        targets = json.loads(saved.read_text())["inventory"]["targets"]
        self.assertEqual([(t["kind"], t["address"], t["port"]) for t in targets],
                         [("compute", "192.0.2.1", 8443), ("unknown", "192.0.2.2", 443)])
        self.assertEqual(output.count("From environment: FLEETINT_INSTALL_BMC_PORT"), 1)

    def test_invalid_environment_is_rejected_without_echoing_its_value(self):
        invalid = (("KUBE_CONTEXT", "\x1b[Hunexpected-context"), ("NAMESPACE", "$(touch SENTINEL)"),
                   ("CHART_VERSION", "latest"))
        for suffix, value in invalid:
            with self.subTest(suffix=suffix), tempfile.TemporaryDirectory() as directory:
                self.directory = Path(directory)
                key = f"FLEETINT_INSTALL_{suffix}"
                code, output, calls, _ = self.run_script(install_env={key: value})
                self.assertNotEqual(code, 0, output)
                self.assertIn(f"Invalid value in {key}", output)
                self.assertNotIn(value, output)
                self.assertEqual(calls, [["helm", "version", "--short"]])

    def test_existing_nico_routes_to_manual_installation_before_cluster_access(self):
        for ui in ("plain", "tui"):
            with self.subTest(ui=ui), tempfile.TemporaryDirectory() as directory:
                self.directory = Path(directory)
                code, output, calls, saved = self.run_script(ui=ui, existing_nico="yes")
                self.assertEqual(code, 0, output)
                self.assertIn("#keep-an-existing-nico-deployment", output)
                self.assertEqual(calls, [["helm", "version", "--short"], CHART_LOOKUP])
                self.assertFalse(saved.exists())

    def test_piped_installer_asks_questions_and_uses_pinned_context_and_version(self):
        code, output, calls, saved = self.run_script()
        self.assertEqual(code, 0, output)
        values = json.loads(saved.read_text())
        self.assertEqual(values["inventory"]["targets"][0]["name"], "$(touch SENTINEL)")
        self.assertFalse(any("create" in c and "secret" in c for c in calls))
        for call in calls:
            if call[0] == "kubectl":
                self.assertEqual(call[call.index("--context") + 1], "test-context")
        upgrade = next(c for c in calls if "upgrade" in c)
        self.assertIn("--wait", upgrade)
        self.assertFalse(any(arg.startswith("--server-side") for arg in upgrade))
        self.assertEqual(upgrade[upgrade.index("--version") + 1], "1.2.3")
        self.assertEqual(upgrade[upgrade.index("--kube-context") + 1], "test-context")

    def test_helm4_installs_with_client_side_apply_and_legacy_wait(self):
        code, output, calls, saved = self.run_script(helm_version="v4.0.0", ui="tui", no_python=True)
        self.assertEqual(code, 0, output)
        self.assertTrue(saved.is_file())
        upgrade = next(c for c in calls if c[:2] == ["helm", "upgrade"])
        self.assertIn("--server-side=false", upgrade)
        self.assertIn("--wait=legacy", upgrade)
        self.assertNotIn("--wait", upgrade)
        self.assertEqual(upgrade[upgrade.index("--timeout") + 1], "10m")
        self.assertTrue(any(c[:2] == ["helm", "status"] for c in calls))

    def test_unsupported_helm_versions_fail_before_questions_or_cluster_access(self):
        for version in ("v2.17.0", "v5.0.0", "unrecognized"):
            with self.subTest(version=version), tempfile.TemporaryDirectory() as directory:
                self.directory = Path(directory)
                code, output, calls, _ = self.run_script(helm_version=version)
                self.assertNotEqual(code, 0, output)
                self.assertEqual(calls, [["helm", "version", "--short"]])
                self.assertNotIn("Kubernetes context", output)

    def test_helm4_declining_confirmation_makes_no_cluster_changes(self):
        code, output, calls, _ = self.run_script(helm_version="v4.3.2+g8346aaa", confirm="no")
        self.assertEqual(code, 0, output)
        self.assertEqual(calls, [["helm", "version", "--short"], CHART_LOOKUP])

    def test_helm4_preflight_failure_makes_no_cluster_changes(self):
        code, _, calls, _ = self.run_script(helm_version="v4.0.0", fail="template")
        self.assertNotEqual(code, 0)
        self.assertFalse(any("create" in c or "apply" in c or "upgrade" in c for c in calls))

    def test_selected_chart_controls_the_token_values_field(self):
        for chart_field in ("enrollmentToken", "agentToken", "both"):
            with self.subTest(chart_field=chart_field), tempfile.TemporaryDirectory() as directory:
                self.directory = Path(directory)
                code, output, calls, saved = self.run_script(chart_token=chart_field)
                self.assertEqual(code, 0, output)
                backend = json.loads(saved.read_text())["backend"]
                expected = "agentToken" if chart_field == "agentToken" else "enrollmentToken"
                self.assertEqual(set(backend), {"url", expected})
                self.assertEqual(backend[expected], {"existingSecret": "fleet-intelligence-oob-collector-agent-token", "secretKey": "agent-token"})
                show = next(c for c in calls if c[:3] == ["helm", "show", "values"])
                self.assertEqual(show[show.index("--version") + 1], "1.2.3")
                self.assertLess(calls.index(show), next(i for i, c in enumerate(calls) if c[:2] == ["helm", "template"]))

    def test_unreadable_chart_defaults_stop_before_cluster_mutations(self):
        code, _, calls, _ = self.run_script(fail="show-values")
        self.assertNotEqual(code, 0)
        self.assertFalse(any("create" in c or "apply" in c or "upgrade" in c for c in calls))

    def test_unsupported_chart_defaults_stop_before_cluster_mutations(self):
        code, _, calls, _ = self.run_script(chart_token="")
        self.assertNotEqual(code, 0)
        self.assertFalse(any("create" in c or "apply" in c or "upgrade" in c for c in calls))

    def test_installs_without_python_or_jq_on_path(self):
        code, output, _, _ = self.run_script(no_python=True)
        self.assertEqual(code, 0, output)

    def test_hardware_choices_collect_all_example_target_fields(self):
        for kind, device in (("compute", "Compute node"), ("nvswitch", "NVSwitch"), ("power_shelf", "Power shelf"), ("unknown", "Device")):
            with self.subTest(kind=kind), tempfile.TemporaryDirectory() as directory:
                self.directory = Path(directory)
                answers = ["test-context", "", "", "1.2.3", "", "zone", "group", "no",
                           kind, "192.0.2.10", "001122334455", "8443", "device-01", "profile",
                           "other-zone", "other-group", "rack", 'rack "12"', "", "no",
                           "existing", "", "", "existing", "", "", "", "yes"]
                code, output, _, saved = self.run_script(answers=answers, real_render=bool(shutil.which("helm")))
                self.assertEqual(code, 0, output)
                target = json.loads(saved.read_text())["inventory"]["targets"][0]
                self.assertEqual(target, {"kind": kind, "address": "192.0.2.10", "mac": "00:11:22:33:44:55",
                                         "port": 8443, "name": "device-01", "credentialsProfile": "profile",
                                         "computeZone": "other-zone", "nodeGroup": "other-group", "labels": {"rack": 'rack "12"'}})
                self.assertLess(output.index("Kind: compute"), output.index(device + " BMC IP"))
                for field in ("BMC MAC", "Redfish HTTPS port", "display name", "credentials profile", "compute zone override", "node group override", "label key"):
                    self.assertIn(device + " " + field, output)

    def test_target_label_limit_includes_automatic_labels(self):
        answers = ["test-context", "", "", "1.2.3", "", "zone", "group", "no",
                   "compute", "192.0.2.10", "001122334455", "", "node", "bmc", "", ""]
        for index in range(28):
            answers += ["custom_" + str(index), "value"]
        answers += ["no", "existing", "", "", "existing", "", "", "", "yes"]
        code, output, _, saved = self.run_script(answers=answers)
        self.assertEqual(code, 0, output)
        self.assertEqual(len(json.loads(saved.read_text())["inventory"]["targets"][0]["labels"]), 28)

    def test_target_labels_retry_invalid_reserved_and_duplicate_keys(self):
        answers = ["test-context", "", "", "1.2.3", "", "", "", "no",
                   "power_shelf", "192.0.2.10", "001122334455", "", "", "", "", "",
                   "bad-key", "oob", "switch_id", "rack", "rack-12", "rack", "site", "", "", "no",
                   "existing", "", "", "existing", "", "", "", "yes"]
        code, output, _, saved = self.run_script(answers=answers)
        self.assertEqual(code, 0, output)
        target = json.loads(saved.read_text())["inventory"]["targets"][0]
        self.assertEqual(target["labels"], {"rack": "rack-12", "site": ""})
        self.assertNotIn("credentialsProfile", target)
        self.assertNotIn("name", target)

    def test_json_values_preserve_quotes_backslashes_and_shell_syntax(self):
        name = 'node "quoted" \\ rack $(touch SENTINEL) `touch SENTINEL`'
        code, output, _, saved = self.run_script(display_name=name)
        self.assertEqual(code, 0, output)
        self.assertEqual(json.loads(saved.read_text())["inventory"]["targets"][0]["name"], name)

    def test_tui_installs_with_arrow_menus_and_restores_terminal(self):
        code, output, _, saved = self.run_script(ui="tui", arrows=True)
        self.assertEqual(code, 0, output)
        self.assertEqual(json.loads(saved.read_text())["inventory"]["targets"][0]["kind"], "compute")
        self.assertNotIn("Value:", output)
        self.assertIn("Review the installation settings", output)
        self.assertIn("Page 2/", output)
        self.assertIn("\x1b[?1049h", output)
        self.assertIn("\x1b[?1049l", output)
        self.assertIn("\x1b[?2004h", output)
        self.assertIn("\x1b[?2004l", output)
        self.assertIn("NVIDIA / FLEET INTELLIGENCE", output)
        self.assertIn("Hardware visibility, beyond the host.", output)
        self.assertIn("\x1b[38;2;118;185;0m", output)

    def test_tui_has_explicit_background_input_and_selection_colors(self):
        code, output, _, _ = self.run_script(ui="tui", arrows=True)
        self.assertEqual(code, 0, output)
        self.assertIn("48;2;16;20;24m", output)
        self.assertIn("48;2;24;32;39m", output)
        self.assertIn("48;2;35;50;28m", output)
        self.assertNotIn("\x1b[2m", output)
        after_ui = output.split("\x1b[?1049l", 1)[1]
        scrollback = re.sub(r"\x1b\[\?1049h.*?\x1b\[\?1049l", "", after_ui, flags=re.DOTALL)
        self.assertNotIn("48;", scrollback)

    def test_tui_centers_the_panel_in_a_wide_terminal(self):
        code, output, _, _ = self.run_script(ui="tui", columns=120)
        self.assertEqual(code, 0, output)
        # A 72-column panel centered in 120 columns starts after 24 spaces.
        plain = re.sub(r"\x1b\[[0-?]*[ -/]*[@-~]", "", output)
        self.assertIn(" " * 24 + "╭" + "─" * 70 + "╮", plain)
        self.assertIn("\x1b[96G", output)

    def test_small_terminal_uses_plain_prompts(self):
        code, output, calls, _ = self.run_script(ui="fallback", columns=60, confirm="no")
        self.assertEqual(code, 0, output)
        self.assertIn("Terminal smaller than 80x24", output)
        self.assertNotIn("\x1b[?1049h", output)
        self.assertEqual(calls, [["helm", "version", "--short"], CHART_LOOKUP])

    def test_forced_tui_rejects_a_short_terminal_before_cluster_changes(self):
        code, output, calls, _ = self.run_script(ui="tui", rows=20)
        self.assertNotEqual(code, 0)
        self.assertIn("80 columns and 24 rows", output)
        self.assertEqual(calls, [["helm", "version", "--short"]])

    def test_tui_palette_falls_back_to_terminal_color_capabilities(self):
        for mode, background in (("", "48;5;233m"), ("ansi", "37;40m")):
            with self.subTest(mode=mode), tempfile.TemporaryDirectory() as directory:
                self.directory = Path(directory)
                code, output, _, _ = self.run_script(ui="tui", color_mode=mode, confirm="no")
                self.assertEqual(code, 0, output)
                self.assertIn(background, output)
                self.assertNotIn("38;2;", output)

    def test_long_helper_text_keeps_the_input_field_on_screen(self):
        body = SCRIPT.split("  ui_mode=auto\n", 1)[0]
        code = body + '\nui_reset=""; ui_frame=""; ui_paragraph "$1"; printf "%s" "$ui_frame"\n}\nfleetint_install "$@"\n'
        result = subprocess.run(["bash", "-c", code, "paragraph", "x" * 1024], text=True, capture_output=True)
        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertLessEqual(len(result.stdout.splitlines()), 2)
        self.assertTrue(result.stdout.rstrip().endswith("..."))

    def test_typing_keeps_field_borders_and_background_inside_the_panel(self):
        for columns in (80, 200):
            with self.subTest(columns=columns), tempfile.TemporaryDirectory() as directory:
                self.directory = Path(directory)
                code, output, calls, _ = self.run_script(ui="tui", columns=columns, ascii_only=True, glyphs="ascii",
                                                       echo=True, confirm="no")
                self.assertEqual(code, 0, output)
                first = output.split("\x1b[H")[1]
                cells = terminal_screen(first, columns)
                lines = ["".join(char for char, _ in line) for line in cells]
                top = next(i for i, line in enumerate(lines) if line.strip() == "+" + "-" * 70 + "+")
                field = cells[top + 1]
                left = (columns - 72) // 2
                right = left + 71
                self.assertEqual(field[left][0], "|")
                self.assertEqual(field[right][0], "|")
                self.assertIn("test-context", lines[top + 1])
                self.assertTrue(all(not shaded for _, shaded in field[:left + 1] + field[right:]))
                self.assertTrue(all(shaded for _, shaded in field[left + 1:right]))
                self.assertEqual(calls, [["helm", "version", "--short"], CHART_LOOKUP])

    def test_input_remains_responsive_after_an_idle_timeout(self):
        code, output, calls, _ = self.run_script(ui="tui", input_delay=1.2, confirm="no", echo=True)
        self.assertEqual(code, 0, output)
        self.assertEqual(calls, [["helm", "version", "--short"], CHART_LOOKUP])

    def test_termination_during_input_restores_terminal_modes(self):
        code, output, calls, _ = self.run_script(ui="tui", input_signal=signal.SIGTERM, input_delay=0.1, echo=True)
        self.assertEqual(code, 143, output)
        self.assertIn("\x1b[?1049l", output)
        self.assertEqual(calls, [["helm", "version", "--short"]])

    def test_missing_stty_uses_plain_prompts_with_a_visible_notice(self):
        code, output, calls, _ = self.run_script(ui="fallback", no_python=True, missing_stty=True, confirm="no")
        self.assertEqual(code, 0, output)
        self.assertIn("stty unavailable; using plain text prompts.", output)
        self.assertNotIn("\x1b[?1049h", output)
        self.assertEqual(calls, [["helm", "version", "--short"], CHART_LOOKUP])

    def test_multiline_paste_is_rejected_without_answering_later_questions(self):
        answers = ["\x1b[200~bad\ncontext\x1b[201~", "demo-cluster", "", "", "1.2.3", "", "zone", "group", "no",
                   "compute", "192.0.2.10", "001122334455", "", "node", "bmc", "", "", "", "no",
                   "existing", "", "", "existing", "", "", "", "yes"]
        code, output, calls, _ = self.run_script(ui="tui", answers=answers, echo=True)
        self.assertEqual(code, 0, output)
        upgrade = next(c for c in calls if c[:2] == ["helm", "upgrade"])
        self.assertEqual(upgrade[upgrade.index("--kube-context") + 1], "demo-cluster")

    def test_no_options_defaults_to_unicode_tui_in_any_locale(self):
        for c_locale in (True, False):
            with self.subTest(c_locale=c_locale), tempfile.TemporaryDirectory() as directory:
                self.directory = Path(directory)
                code, output, calls, _ = self.run_script(ui="auto", ascii_only=c_locale,
                                                       terminal="xterm-256color", confirm="no")
                self.assertEqual(code, 0, output)
                self.assertIn("\x1b[?1049h", output)
                self.assertIn("╭───╮", output)
                self.assertIn("● Cluster", output)
                self.assertNotIn("[===]", output)
                self.assertEqual(calls, [["helm", "version", "--short"], CHART_LOOKUP])

    def test_unicode_override_works_with_a_c_locale_and_generic_terminal(self):
        code, output, calls, _ = self.run_script(ui="tui", glyphs="unicode", ascii_only=True,
                                               terminal="xterm-256color", confirm="no", no_color=True)
        self.assertEqual(code, 0, output)
        self.assertIn("╭───╮", output)
        self.assertIn("● Cluster", output)
        self.assertNotIn("[===]", output)
        self.assertNotIn("48;", output)
        self.assertEqual(calls, [["helm", "version", "--short"], CHART_LOOKUP])

    def test_ascii_override_works_with_a_utf8_locale(self):
        code, output, calls, _ = self.run_script(ui="tui", glyphs="ascii", terminal="xterm-ghostty", confirm="no")
        self.assertEqual(code, 0, output)
        output.encode("ascii")
        self.assertIn("[===]", output)
        self.assertIn("\x1b[?1049h", output)
        self.assertEqual(calls, [["helm", "version", "--short"], CHART_LOOKUP])

    def test_unicode_override_does_not_force_tui_or_override_plain_mode(self):
        code, output, calls, _ = self.run_script(ui="plain", glyphs="unicode", confirm="no")
        self.assertEqual(code, 0, output)
        self.assertNotIn("\x1b", output)
        self.assertEqual(calls, [["helm", "version", "--short"], CHART_LOOKUP])

    def test_ghostty_uses_unicode_borders_with_a_c_locale(self):
        code, output, calls, _ = self.run_script(ui="tui", terminal="xterm-ghostty", ascii_only=True, color_mode="", confirm="no")
        self.assertEqual(code, 0, output)
        self.assertIn("╭───╮", output)
        self.assertIn("● Cluster", output)
        self.assertIn("48;2;16;20;24m", output)
        self.assertEqual(calls, [["helm", "version", "--short"], CHART_LOOKUP])

    def test_field_editing_preserves_the_submitted_value(self):
        cases = (("demo-clustr\x1b[De\x1b[F", "demo-cluster"),
                 ("discard\x15cluster", "cluster"),
                 ("abc\x1b[H\x1b[3~\x1b[F\x7fZ", "bZ"),
                 ("réseau🙂\x7f", "réseau"),
                 ("\x1b[200~demo-cluster\x1b[201~", "demo-cluster"),
                 ("x" * 120, "x" * 120))
        for typed, expected in cases:
            with self.subTest(typed=typed), tempfile.TemporaryDirectory() as directory:
                self.directory = Path(directory)
                answers = [typed, "", "", "1.2.3", "", "zone", "group", "no",
                           "compute", "192.0.2.10", "001122334455", "", "node", "bmc", "", "", "", "no",
                           "existing", "", "", "existing", "", "", "", "yes"]
                code, output, calls, _ = self.run_script(ui="tui", answers=answers, echo=True)
                self.assertEqual(code, 0, output)
                upgrade = next(c for c in calls if c[:2] == ["helm", "upgrade"])
                self.assertEqual(upgrade[upgrade.index("--kube-context") + 1], expected)

    def test_tui_auto_detection_needs_no_additional_tools(self):
        code, output, _, _ = self.run_script(ui="auto", no_python=True)
        self.assertEqual(code, 0, output)
        self.assertIn("\x1b[?1049h", output)

    def test_tui_escape_cancels_before_cluster_changes(self):
        code, output, calls, _ = self.run_script(ui="tui", cancel_at=8)
        self.assertNotEqual(code, 0)
        self.assertIn("\x1b[?1049l", output)
        self.assertFalse(any(call[0] == "kubectl" for call in calls))

    def test_tui_ctrl_c_restores_terminal(self):
        code, output, calls, _ = self.run_script(ui="tui", cancel_at=0, input_delay=0.1)
        self.assertEqual(code, 130, output)
        self.assertIn("\x1b[?1049l", output)
        self.assertFalse(any(call[0] == "kubectl" for call in calls))

    def test_tui_declining_confirmation_makes_no_cluster_changes(self):
        code, output, calls, _ = self.run_script(ui="tui", confirm="no")
        self.assertEqual(code, 0, output)
        self.assertFalse(any(call[0] == "kubectl" for call in calls))
        self.assertIn("\x1b[?1049l", output)

    def test_tui_failure_restores_terminal(self):
        code, output, _, _ = self.run_script(ui="tui", mode="file", fail="missing-file")
        self.assertNotEqual(code, 0)
        self.assertIn("\x1b[?1049l", output)

    def test_tui_no_color_removes_color_styles(self):
        code, output, _, _ = self.run_script(ui="tui", no_color=True)
        self.assertEqual(code, 0, output)
        self.assertNotIn("\x1b[38;2;118;185;0m", output)
        self.assertNotIn("\x1b[1m", output)
        self.assertNotIn("\x1b[2m", output)
        self.assertNotIn("48;", output)

    def test_tui_ascii_option_preserves_navigation(self):
        code, output, _, _ = self.run_script(ui="tui", ascii_only=True, glyphs="ascii")
        self.assertEqual(code, 0, output)
        output.encode("ascii")
        self.assertIn("[OK]  NICo health Deployment ready", output.replace("\x1b[0m", ""))

    def test_tui_install_failure_does_not_report_completion(self):
        code, output, _, _ = self.run_script(ui="tui", fail="upgrade")
        self.assertNotEqual(code, 0)
        self.assertIn("[3/5]", output)
        self.assertNotIn("[done]", output)

    def test_namespace_must_be_active_before_installing(self):
        code, output, calls, _ = self.run_script(fail="namespace-wait")
        self.assertNotEqual(code, 0, output)
        self.assertIn("namespace never became Active", output)
        self.assertFalse(any(call[:3] == ["helm", "upgrade", "--install"] for call in calls))
        self.assertNotIn("[done]", output)

    def test_installer_waits_for_all_rollouts_and_an_accepted_upload(self):
        code, output, calls, _ = self.run_script(
            ui="tui", fail="inventory-retry", result_keys=("2", "\x1b[B", "\x1b[C", "\x1b[C", "\x1b[C", "1", "\n"),
        )
        self.assertEqual(code, 0, output)
        rollouts = [i for i, call in enumerate(calls) if "rollout" in call]
        polls = [i for i, call in enumerate(calls) if "--tail=500" in call]
        self.assertEqual([calls[i][7] for i in rollouts], [
            "deployment.apps/fixture-collector", "deployment.apps/fixture-nico", "deployment.apps/fixture-gateway",
        ])
        self.assertEqual(len(polls), 2)
        self.assertLess(rollouts[-1], polls[0])
        self.assertIn("first inventory upload accepted in this cycle", output)
        self.assertIn("OTLP export failed: connection timeout", output)
        self.assertIn("[done] ", output.replace("\x1b[0m", ""))
        self.assertIn("Confirm fresh inventory and health telemetry", output)
        result_panel = output.split("\x1b[?1049h")[-1].split("\x1b[?1049l")[0]
        self.assertIn("Verification complete", result_panel)
        self.assertIn("Recent warnings are available in Logs.", result_panel)
        self.assertIn("Page 2/", result_panel)
        self.assertIn("Collector logs", result_panel)
        self.assertIn("Gateway logs", result_panel)
        self.assertIn("OTLP export failed", result_panel)
        self.assertEqual(sum("--tail=500" in call for call in calls), 2)
        post_install = output.split("\x1b[?1049l")[-1]
        self.assertEqual(re.findall(r"\x1b\[38;2;118;185;0m✓\x1b\[0m  (.*)\r", post_install), [
            "Kubernetes API reachable and Helm chart rendered",
            "Namespace active and required Secret keys present",
            "Helm release applied",
            "OOB collector Deployment ready",
            "NICo health Deployment ready",
            "Telemetry gateway Deployment ready",
            "Fleet Intelligence accepted an inventory upload",
        ])

    def test_missing_deployment_prevents_success_after_helm(self):
        code, output, calls, _ = self.run_script(fail="missing-nico")
        self.assertNotEqual(code, 0, output)
        self.assertIn("Expected one NICo health Deployment", output)
        self.assertTrue(any(call[:3] == ["helm", "upgrade", "--install"] for call in calls))
        self.assertNotIn("[done]", output)

    def test_rollout_failure_displays_release_events_and_relevant_logs(self):
        code, output, calls, _ = self.run_script(
            ui="tui", fail="rollout", result_keys=("2", "\x1b[C", "\x1b[C", "\n"),
        )
        self.assertNotEqual(code, 0, output)
        self.assertIn("Init:CrashLoopBackOff", output)
        self.assertIn("FailedMount", output)
        self.assertIn("Error: invalid NICo configuration", output)
        self.assertIn("OTLP export failed: connection timeout", output)
        self.assertIn("Scoped collector log", output)
        self.assertNotIn("\x1b]0;", output)
        self.assertNotIn("\x07", output)
        result_panel = output.split("\x1b[?1049h")[-1].split("\x1b[?1049l")[0]
        self.assertIn("Verification needs attention", result_panel)
        self.assertIn("NICo configuration logs", result_panel)
        self.assertIn("Error: invalid NICo configuration", result_panel)
        self.assertIn("✓\x1b[0m  OOB collector Deployment ready", output)
        self.assertNotIn("✓\x1b[0m  NICo health Deployment ready", output)
        self.assertNotIn("✓\x1b[0m  Fleet Intelligence accepted an inventory upload", output)
        self.assertTrue(any("--previous" in call and "collector" in call for call in calls))
        for call in (call for call in calls if "events" in call):
            self.assertIn("involvedObject.uid=release-pod-uid,type=Warning", call)
        self.assertNotIn("[done]", output)

    def test_helm_success_without_an_accepted_upload_is_unverified(self):
        code, output, calls, _ = self.run_script(fail="inventory-timeout")
        self.assertNotEqual(code, 0, output)
        self.assertIn("Could not confirm an accepted inventory upload.", output)
        self.assertIn("BMC login failed", output)
        self.assertIn("Collector still has no confirmed inventory upload", output)
        self.assertGreater(sum("--tail=500" in call for call in calls), 1)
        self.assertNotIn("[done]", output)

    def test_forbidden_inventory_logs_explain_the_stalled_verification(self):
        code, output, calls, _ = self.run_script(fail="logs-forbidden")
        self.assertNotEqual(code, 0, output)
        self.assertIn('cannot get resource "pods/log"', output)
        self.assertEqual(sum("--tail=500" in call for call in calls), 1)
        self.assertNotIn("[done]", output)

    def test_older_positive_inventory_batch_is_accepted(self):
        code, output, _, _ = self.run_script(fail="legacy-inventory")
        self.assertEqual(code, 0, output)
        self.assertIn("inventory mini-batch export completed exportable_count=1", output)
        self.assertIn("[done]", output)

    def test_forced_tui_rejects_unsupported_terminal(self):
        code, output, calls, _ = self.run_script(ui="unsupported")
        self.assertNotEqual(code, 0)
        self.assertIn("Use --plain", output)
        self.assertFalse(any(call[0] == "kubectl" for call in calls))

    def test_review_wrap_preserves_utf8_and_spacing(self):
        text = "x" * 67 + "é" + "測試" * 30 + "  end"
        body = SCRIPT.split("  ui_mode=auto\n", 1)[0]
        code = body + '\nreview_lines=(); review_add "$1"; printf "%s\\0" "${review_lines[@]}"\n}\nfleetint_install "$@"\n'
        result = subprocess.run(["bash", "-c", code, "review", text], text=True, capture_output=True)
        self.assertEqual(result.returncode, 0, result.stderr)
        lines = result.stdout.rstrip("\0").split("\0")
        self.assertEqual("".join(lines), text)
        self.assertTrue(all(len(line.encode()) <= 68 for line in lines))

    def test_dumb_terminal_uses_plain_prompts_with_notice(self):
        code, output, _, _ = self.run_script(ui="fallback")
        self.assertEqual(code, 0, output)
        self.assertIn("using plain text prompts", output)
        self.assertNotIn("\x1b[?1049h", output)

    def test_file_sources_create_missing_secrets(self):
        code, output, calls, _ = self.run_script(mode="file")
        self.assertEqual(code, 0, output)
        self.assertEqual(len([c for c in calls if "create" in c and "secret" in c]), 2)

    def test_declining_confirmation_makes_no_cluster_changes(self):
        code, output, calls, _ = self.run_script(confirm="no", install_env={
            "FLEETINT_INSTALL_NAMESPACE": "env-ns",
            "FLEETINT_INSTALL_CONFIRM": "yes",
        })
        self.assertEqual(code, 0, output)
        self.assertIn("From environment: FLEETINT_INSTALL_NAMESPACE", output)
        self.assertFalse(any(c[0] == "kubectl" or "upgrade" in c for c in calls))

    def test_preflight_failure_makes_no_cluster_changes(self):
        code, _, calls, _ = self.run_script(fail="template")
        self.assertNotEqual(code, 0)
        self.assertFalse(any("create" in c or "apply" in c or "upgrade" in c for c in calls))

    def test_secret_read_failure_never_creates_or_installs(self):
        code, _, calls, _ = self.run_script(mode="file", fail="secret-read")
        self.assertNotEqual(code, 0)
        self.assertFalse(any(("secret" in c and "create" in c) or "upgrade" in c for c in calls))

    def test_empty_secret_key_stops_install(self):
        code, _, calls, _ = self.run_script(fail="empty-key")
        self.assertNotEqual(code, 0)
        self.assertFalse(any("upgrade" in c for c in calls))

    def test_helm_failure_cleans_up(self):
        code, _, calls, _ = self.run_script(fail="upgrade")
        self.assertNotEqual(code, 0)
        self.assertFalse(any("status" in c for c in calls))

    def test_termination_cleans_up_and_reports_partial_install(self):
        code, output, calls, _ = self.run_script(fail="terminate")
        self.assertNotEqual(code, 0)
        self.assertIn("resources already created remain", output)
        self.assertFalse(any("status" in c for c in calls))

    def test_answers_render_with_real_chart(self):
        if not shutil.which("helm"):
            self.skipTest("Helm is unavailable")
        version = subprocess.run(["helm", "version", "--short"], check=True, text=True, capture_output=True).stdout.strip()
        code, output, calls, _ = self.run_script(real_render=True, helm_version=version)
        self.assertEqual(code, 0, output)
        upgrade = next(c for c in calls if c[:2] == ["helm", "upgrade"])
        result = subprocess.run(["helm", "upgrade", "--help", *[arg for arg in upgrade if arg.startswith(("--wait", "--server-side"))]], text=True, capture_output=True)
        self.assertEqual(result.returncode, 0, result.stderr)

    def test_missing_terminal_fails_before_cluster_access(self):
        result = subprocess.run(["bash"], input=SCRIPT, text=True, capture_output=True, start_new_session=True)
        self.assertNotEqual(result.returncode, 0)
        self.assertIn("interactive terminal", result.stderr)

    def test_shell_syntax(self):
        result = subprocess.run(["bash", "-n"], input=SCRIPT, text=True, capture_output=True)
        self.assertEqual(result.returncode, 0, result.stderr)

    def test_help_does_not_need_a_terminal(self):
        result = subprocess.run(["bash", "-s", "--", "--help"], input=SCRIPT, text=True, capture_output=True, start_new_session=True)
        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertIn("--plain", result.stdout)
        self.assertIn("--unicode", result.stdout)
        self.assertIn("--ascii", result.stdout)


if __name__ == "__main__":
    unittest.main()
