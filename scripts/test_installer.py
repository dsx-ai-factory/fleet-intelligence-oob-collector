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
        answers = ["test-context", "", "", "1.2.3", "", "zone", "group",
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

    def run_script(self, *, mode="existing", fail="", confirm="yes", real_render=False, answers=None, no_python=False, display_name="$(touch SENTINEL)", ui="plain", arrows=False, cancel_at=None, no_color=False, ascii_only=False, helm_version="v3.19.0", columns=80, rows=24, color_mode="truecolor", echo=False, terminal=None, input_delay=0, input_signal=None, missing_stty=False, glyphs=None, chart_token="enrollmentToken"):
        mock = self.directory / "mock.py"
        mock.write_text("#!" + sys.executable + "\n" + '''import json, os, pathlib, signal, subprocess, sys, time
name, args = pathlib.Path(sys.argv[0]).name, sys.argv[1:]
with open(os.environ['CALL_LOG'], 'a') as log:
    log.write(json.dumps([name, *args]) + '\\n')
mode, failure = os.environ['TEST_MODE'], os.environ['FAIL_AT']
if name == 'helm':
    if args == ['version', '--short']:
        print(os.environ['TEST_HELM_VERSION'])
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
''')
        mock.chmod(0o700)
        for name in ("helm", "kubectl"):
            (self.directory / name).symlink_to(mock)
        log, saved = self.directory / "calls.jsonl", self.directory / "saved-values.json"
        private_file = self.directory / "private input $(touch SENTINEL)"
        private_file.write_text("fixture-secret-never-print")
        if fail == "missing-file":
            private_file.unlink()
        if answers is None:
            answers = ["test-context", "", "", "1.2.3", "", "zone", "group",
                       "compute", "192.0.2.10", "001122334455", "", display_name, "bmc", "", "", "", "no"]
            for _ in range(2):
                answers += [mode, "", ""] + ([str(private_file)] if mode == "file" else [])
            answers += ["", confirm]
        if no_python:
            for name in ("mktemp", "chmod", "rm", "stty"):
                if name == "stty" and missing_stty:
                    continue
                (self.directory / name).symlink_to(shutil.which(name))
        env = dict(os.environ, PATH=str(self.directory) + os.pathsep + os.environ["PATH"],
                   CALL_LOG=str(log), TEST_MODE=mode, FAIL_AT=fail, SAVED_VALUES=str(saved), TEST_HELM_VERSION=helm_version,
                   CHART_DEFAULTS="backend:\n" + "".join("  " + field + ":\n    existingSecret: fixture\n" for field in (["agentToken", "enrollmentToken"] if chart_token == "both" else [chart_token] if chart_token else [])),
                   TMPDIR=str(self.directory), REAL_RENDER="yes" if real_render else "",
                   REAL_HELM=shutil.which("helm") or "", CHART=str(ROOT / "deployments/helm/fleet-intelligence-oob-collector"))
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
                    if screen.endswith(b"Press Enter to try again."):
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
                        else:
                            if b"Kind: compute" in screen or b"Choose the hardware type" in screen:
                                choices = ["unknown", "compute", "nvswitch", "power_shelf"]
                            elif b"Source: existing or file" in screen or b"How should we read the credentials?" in screen:
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
            self.assertEqual(termios.tcgetattr(master)[3], settings[3], "terminal flags must be restored")
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
        self.assertEqual(calls, [["helm", "version", "--short"]])

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
                answers = ["test-context", "", "", "1.2.3", "", "zone", "group",
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
        answers = ["test-context", "", "", "1.2.3", "", "zone", "group",
                   "compute", "192.0.2.10", "001122334455", "", "node", "bmc", "", ""]
        for index in range(28):
            answers += ["custom_" + str(index), "value"]
        answers += ["no", "existing", "", "", "existing", "", "", "", "yes"]
        code, output, _, saved = self.run_script(answers=answers)
        self.assertEqual(code, 0, output)
        self.assertEqual(len(json.loads(saved.read_text())["inventory"]["targets"][0]["labels"]), 28)

    def test_target_labels_retry_invalid_reserved_and_duplicate_keys(self):
        answers = ["test-context", "", "", "1.2.3", "", "", "",
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
        self.assertNotIn("48;", after_ui)

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
        self.assertEqual(calls, [["helm", "version", "--short"]])

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
                self.assertEqual(calls, [["helm", "version", "--short"]])

    def test_input_remains_responsive_after_an_idle_timeout(self):
        code, output, calls, _ = self.run_script(ui="tui", input_delay=1.2, confirm="no", echo=True)
        self.assertEqual(code, 0, output)
        self.assertEqual(calls, [["helm", "version", "--short"]])

    def test_termination_during_input_restores_terminal_modes(self):
        code, output, calls, _ = self.run_script(ui="tui", input_signal=signal.SIGTERM, echo=True)
        self.assertEqual(code, 143, output)
        self.assertIn("\x1b[?1049l", output)
        self.assertEqual(calls, [["helm", "version", "--short"]])

    def test_missing_stty_uses_plain_prompts_with_a_visible_notice(self):
        code, output, calls, _ = self.run_script(ui="fallback", no_python=True, missing_stty=True, confirm="no")
        self.assertEqual(code, 0, output)
        self.assertIn("stty unavailable; using plain text prompts.", output)
        self.assertNotIn("\x1b[?1049h", output)
        self.assertEqual(calls, [["helm", "version", "--short"]])

    def test_multiline_paste_is_rejected_without_answering_later_questions(self):
        answers = ["\x1b[200~bad\ncontext\x1b[201~", "demo-cluster", "", "", "1.2.3", "", "zone", "group",
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
                self.assertEqual(calls, [["helm", "version", "--short"]])

    def test_unicode_override_works_with_a_c_locale_and_generic_terminal(self):
        code, output, calls, _ = self.run_script(ui="tui", glyphs="unicode", ascii_only=True,
                                               terminal="xterm-256color", confirm="no", no_color=True)
        self.assertEqual(code, 0, output)
        self.assertIn("╭───╮", output)
        self.assertIn("● Cluster", output)
        self.assertNotIn("[===]", output)
        self.assertNotIn("48;", output)
        self.assertEqual(calls, [["helm", "version", "--short"]])

    def test_ascii_override_works_with_a_utf8_locale(self):
        code, output, calls, _ = self.run_script(ui="tui", glyphs="ascii", terminal="xterm-ghostty", confirm="no")
        self.assertEqual(code, 0, output)
        output.encode("ascii")
        self.assertIn("[===]", output)
        self.assertIn("\x1b[?1049h", output)
        self.assertEqual(calls, [["helm", "version", "--short"]])

    def test_unicode_override_does_not_force_tui_or_override_plain_mode(self):
        code, output, calls, _ = self.run_script(ui="plain", glyphs="unicode", confirm="no")
        self.assertEqual(code, 0, output)
        self.assertNotIn("\x1b", output)
        self.assertEqual(calls, [["helm", "version", "--short"]])

    def test_ghostty_uses_unicode_borders_with_a_c_locale(self):
        code, output, calls, _ = self.run_script(ui="tui", terminal="xterm-ghostty", ascii_only=True, color_mode="", confirm="no")
        self.assertEqual(code, 0, output)
        self.assertIn("╭───╮", output)
        self.assertIn("● Cluster", output)
        self.assertIn("48;2;16;20;24m", output)
        self.assertEqual(calls, [["helm", "version", "--short"]])

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
                answers = [typed, "", "", "1.2.3", "", "zone", "group",
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
        code, output, calls, _ = self.run_script(ui="tui", cancel_at=7)
        self.assertNotEqual(code, 0)
        self.assertIn("\x1b[?1049l", output)
        self.assertFalse(any(call[0] == "kubectl" for call in calls))

    def test_tui_ctrl_c_restores_terminal(self):
        code, output, calls, _ = self.run_script(ui="tui", cancel_at=0)
        self.assertNotEqual(code, 0)
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

    def test_tui_install_failure_does_not_report_completion(self):
        code, output, _, _ = self.run_script(ui="tui", fail="upgrade")
        self.assertNotEqual(code, 0)
        self.assertIn("[3/4]", output)
        self.assertNotIn("[done]", output)

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
        code, output, calls, _ = self.run_script(confirm="no")
        self.assertEqual(code, 0, output)
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
