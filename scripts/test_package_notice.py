# Copyright (c) 2026, NVIDIA CORPORATION & AFFILIATES. All rights reserved.
# SPDX-License-Identifier: Apache-2.0
import importlib.util
import json
from pathlib import Path
import tempfile
from types import SimpleNamespace
import unittest
from unittest.mock import patch

spec = importlib.util.spec_from_file_location("notice", Path(__file__).with_name("generate-notice.py"))
notice = importlib.util.module_from_spec(spec)
spec.loader.exec_module(notice)


class PackageNoticeTests(unittest.TestCase):
    def test_scope_and_missing_terms(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            (root / "LICENSE").write_text("Required copyright and license text")
            packages = [{"name": name, "version": "1.0.0", "source": "registry",
                         "license": "MIT", "manifest_path": str(root / "Cargo.toml")}
                        for name in ("selected", "unrelated")]
            with patch.object(notice.subprocess, "run", return_value=SimpleNamespace(
                    stdout=json.dumps({"packages": packages}))), patch.object(
                    notice.subprocess, "check_output", return_value="selected v1.0.0\n"):
                result = notice.generate("carbide-health", "aarch64-unknown-linux-musl", directory)
                self.assertIn("Required copyright and license text", result)
                self.assertIn("selected (1.0.0)", result)
                self.assertNotIn("unrelated", result)
                (root / "LICENSE").unlink()
                with self.assertRaises(SystemExit):
                    notice.generate("carbide-health", "aarch64-unknown-linux-musl", directory)

    def test_collector_refresh_preserves_bundled_notices(self):
        with tempfile.TemporaryDirectory() as directory:
            output = Path(directory) / "NOTICE"
            output.write_text("Old collector\n" + notice.BUNDLED_MARKER + "Pinned component notices\n")
            with patch.object(notice, "generate", return_value="Updated collector\n"), patch(
                    "sys.argv", ["generate-notice.py", "--output", str(output)]):
                notice.main()
            self.assertEqual(output.read_text(),
                             "Updated collector\n" + notice.BUNDLED_MARKER + "Pinned component notices\n")
