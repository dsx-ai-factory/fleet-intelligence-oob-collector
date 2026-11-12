# Copyright (c) 2026, NVIDIA CORPORATION & AFFILIATES. All rights reserved.
# SPDX-License-Identifier: Apache-2.0
import importlib.util
from pathlib import Path
import re
import tempfile
import unittest

spec = importlib.util.spec_from_file_location("notice_check", Path(__file__).with_name("check-notice.py"))
notice = importlib.util.module_from_spec(spec)
spec.loader.exec_module(notice)
TEXT = (notice.ROOT / "NOTICE").read_text()


class NoticeTests(unittest.TestCase):
    def test_current_notice(self):
        self.assertEqual(notice.check_notice(TEXT), [])

    def copy_inputs(self, root):
        for name in ("Cargo.lock", "Makefile", ".github/workflows/release.yml"):
            target = root / name
            target.parent.mkdir(parents=True, exist_ok=True)
            target.write_bytes((notice.ROOT / name).read_bytes())

    def test_new_external_dependency_requires_notice(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            self.copy_inputs(root)
            lock = root / "Cargo.lock"
            lock.write_text(lock.read_text() + '\n[[package]]\nname = "new-external-crate"\n'
                            'version = "1.2.3"\nsource = "registry+https://github.com/rust-lang/crates.io-index"\n')
            errors = notice.check_notice(TEXT, root)
            self.assertIn("Missing NOTICE entry for locked dependency: new-external-crate (1.2.3)", errors)
            updated = TEXT.replace("\nNICo hardware health (carbide-health)\n",
                "\nnew-external-crate (1.2.3)\nLicense: MIT\nCopyright Example contributors\n"
                "[See shared license text M02 below.]\n\nNICo hardware health (carbide-health)\n", 1)
            self.assertEqual(notice.check_notice(updated, root), [])

    def test_multiple_locked_versions_require_separate_entries(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            self.copy_inputs(root)
            lock = root / "Cargo.lock"
            lock.write_text(lock.read_text() + '\n[[package]]\nname = "aho-corasick"\n'
                            'version = "999.0.0"\nsource = "git+https://example.test/source#1234"\n')
            self.assertIn("Missing NOTICE entry for locked dependency: aho-corasick (999.0.0)",
                          notice.check_notice(TEXT, root))

    def test_version_bump_requires_new_entry(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            self.copy_inputs(root)
            lock = root / "Cargo.lock"
            old = 'name = "aho-corasick"\nversion = "1.1.4"'
            self.assertIn(old, lock.read_text())
            lock.write_text(lock.read_text().replace(old, 'name = "aho-corasick"\nversion = "999.0.0"'))
            self.assertIn("Missing NOTICE entry for locked dependency: aho-corasick (999.0.0)",
                          notice.check_notice(TEXT, root))

    def test_local_packages_and_formatting_do_not_require_notice_updates(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            self.copy_inputs(root)
            lock = root / "Cargo.lock"
            lock.write_text("# harmless formatting/comment change\n" + lock.read_text()
                            + '\n[[package]]\nname = "local-workspace-member"\nversion = "1.0.0"\n')
            makefile = root / "Makefile"
            makefile.write_text(makefile.read_text() + "\n# unrelated build/test edit\n")
            (root / "Dockerfile").write_text("# unrelated image edit\n")
            self.assertEqual(notice.check_notice(TEXT, root), [])

    def test_missing_dependency_entry_is_not_satisfied_by_references(self):
        text = TEXT.replace("aho-corasick (1.1.4)\nLicense:", "removed-entry (1.1.4)\nLicense:")
        self.assertIn("Missing NOTICE entry for locked dependency: aho-corasick (1.1.4)",
                      notice.check_notice(text))

    def test_source_pin_requires_matching_revision(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            self.copy_inputs(root)
            nico, agent = notice.source_pins(root)
            for name, pin in [("Makefile", agent), (".github/workflows/release.yml", nico)]:
                with self.subTest(pin=name):
                    target = root / name
                    original = target.read_text()
                    target.write_text(original.replace(pin, "1" * 40))
                    self.assertTrue(any("revision" in error for error in notice.check_notice(TEXT, root)))
                    target.write_text(original)

    def test_deleted_component_sections(self):
        for index, heading in enumerate(notice.COMPONENTS[:4]):
            with self.subTest(component=heading):
                start = TEXT.index("\n" + heading + "\n")
                end = TEXT.index("\n" + notice.COMPONENTS[index + 1] + "\n")
                self.assertTrue(notice.check_notice(TEXT[:start] + TEXT[end:]))

    def test_component_heading_alone_is_insufficient(self):
        for index, heading in enumerate(notice.COMPONENTS[:4]):
            with self.subTest(component=heading):
                start = TEXT.index("\n" + heading + "\n") + len(heading) + 2
                end = TEXT.index("\n" + notice.COMPONENTS[index + 1] + "\n")
                self.assertTrue(notice.check_notice(TEXT[:start] + TEXT[end:]))

    def test_broken_license_reference(self):
        self.assertTrue(notice.check_notice(TEXT.replace(
            "[See shared license text A01 below.]", "[See shared license text Z999 below.]", 1)))

    def test_missing_or_duplicate_definition(self):
        heading = "Shared license text A01\n\n"
        self.assertTrue(notice.check_notice(TEXT.replace(heading, "", 1)))
        self.assertTrue(notice.check_notice(TEXT + "\n" + heading + "duplicate text\n"))

    def test_empty_definition(self):
        matches = list(re.finditer(r"^Shared license text [AM][0-9]+\n\n", TEXT, re.M))
        self.assertTrue(notice.check_notice(TEXT[:matches[0].end()] + TEXT[matches[1].start():]))

    def test_wrong_dependency_label(self):
        self.assertTrue(notice.check_notice(TEXT.replace(
            "[See shared dependency notice D001: aho-corasick (1.1.4).]",
            "[See shared dependency notice D001: wrong-dependency (1.1.4).]", 1)))

    def test_malformed_reference(self):
        self.assertTrue(notice.check_notice(TEXT.replace(
            "[See shared license text A01 below.]", "[See shared license A01 below.]", 1)))

    def test_incomplete_reference_with_another_valid_use(self):
        for reference in (
            "[See shared license text A01 below.]",
            "[See shared dependency notice D001: aho-corasick (1.1.4).]",
        ):
            with self.subTest(reference=reference):
                self.assertGreater(TEXT.count(reference), 1)
                text = TEXT.replace(reference, reference[:-1], 1)
                self.assertIn(reference, text)
                self.assertIn("Malformed shared notice/license reference", notice.check_notice(text))

    def test_unfinished_shared_marker(self):
        self.assertIn("Malformed shared notice/license reference",
                      notice.check_notice(TEXT + "\n[See shared"))

    def test_nested_license_notices(self):
        # Retain the nested notices found in the published crate audit, including
        # both bundled regex-syntax versions' Unicode data license.
        required = {
            ("aws-lc-sys", "0.47.0"): ("aws-lc/third_party/fiat/LICENSE",),
            ("petgraph", "0.8.3"): ("assets/images/LICENSE.md",),
            ("regex-syntax", "0.8.10"): ("src/unicode_tables/LICENSE-UNICODE",),
            ("regex-syntax", "0.8.11"): ("src/unicode_tables/LICENSE-UNICODE",),
            ("ring", "0.17.14"): ("src/polyfill/once_cell/LICENSE-APACHE",
                                  "src/polyfill/once_cell/LICENSE-MIT", "third_party/fiat/LICENSE"),
            ("tracing-core", "0.1.36"): ("src/spin/LICENSE",),
        }
        headers = list(re.finditer(r"^([-A-Za-z0-9_]+) \(([0-9][^\n]*)\)\nLicense: ", TEXT, re.M))
        for key, paths in required.items():
            entries = [TEXT[header.end():headers[index + 1].start() if index + 1 < len(headers) else len(TEXT)]
                       for index, header in enumerate(headers) if (header[1], header[2]) == key]
            self.assertTrue(entries, key)
            for entry in entries:
                entry = re.split(r"^={57}$|^Shared dependency notice D[0-9]+$", entry, maxsplit=1, flags=re.M)[0]
                for path in paths:
                    with self.subTest(dependency=key, path=path):
                        self.assertIn(f"[{path}]\n", entry)
                if key[0] == "regex-syntax":
                    self.assertIn("Copyright © 1991-2018 Unicode, Inc. All rights reserved.", entry)
                    self.assertIn("UNICODE, INC. LICENSE AGREEMENT - DATA FILES AND SOFTWARE", entry)
                    self.assertIn("written authorization of the copyright holder.", entry)

    def test_recursive_license_reference(self):
        self.assertTrue(notice.check_notice(TEXT.replace(
            "Shared license text A01\n\n", "Shared license text A01\n\n[See shared license text A01 below.]\n", 1)))
