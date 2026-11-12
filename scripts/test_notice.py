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


class CrateSourceAuditTests(unittest.TestCase):
    def setUp(self):
        directory = tempfile.TemporaryDirectory()
        self.addCleanup(directory.cleanup)
        self.root = Path(directory.name)
        self.crate = self.root / "example-1.2.3"
        self.crate.mkdir()
        self.manifest = '[package]\nname = "example"\nversion = "1.2.3"\n'
        (self.crate / "Cargo.toml").write_text(self.manifest)
        self.license = "Copyright Example contributors.\nPermission is hereby granted to use example.\n"
        (self.crate / "LICENSE").write_text(self.license)
        self.text = "example (1.2.3)\nLicense: MIT\n" + self.license

    def audit(self, text=None):
        return notice.audit_crate_sources(self.text if text is None else text, [self.root])

    def test_nested_terms_are_required_not_just_the_path(self):
        path = self.crate / "src/unicode_tables/LICENSE-UNICODE"
        path.parent.mkdir(parents=True)
        terms = "Copyright Unicode contributors.\nPermission to use the Unicode data.\n"
        path.write_text(terms)
        for text in (self.text, self.text + "\n[src/unicode_tables/LICENSE-UNICODE]\n"):
            with self.subTest(text=text):
                self.assertIn("Missing source license/attribution text in NOTICE: "
                              "example-1.2.3/src/unicode_tables/LICENSE-UNICODE", self.audit(text))
        self.assertEqual(self.audit(self.text + terms), [])

    def test_full_grant_copyright_and_disclaimer_must_remain(self):
        terms = "Copyright © Example.\nPermission to use.\nProvided without warranty.\n"
        (self.crate / "LICENSE").write_text(terms, encoding="utf-8")
        for line in terms.splitlines(keepends=True):
            with self.subTest(removed=line):
                self.assertTrue(self.audit("example (1.2.3)\nLicense: MIT\n" + terms.replace(line, "")))
        self.assertEqual(self.audit("example (1.2.3)\nLicense: MIT\n" + terms.replace("\n", "\r\n\n")), [])

    def test_shared_license_expansion(self):
        text = "example (1.2.3)\nLicense: MIT\n[See shared license text M02 below.]\n\n"
        text += "SHARED LICENSE TEXTS\nShared license text M02\n\n" + self.license
        self.assertEqual(self.audit(text), [])

    def test_other_dependency_cannot_supply_missing_terms(self):
        text = "example (1.2.3)\nLicense: MIT\n[LICENSE]\n" + "=" * 57 + "\n\n"
        text += "other (9.0.0)\nLicense: MIT\n" + self.license
        errors = self.audit(text)
        self.assertIn("Missing source license/attribution text in NOTICE: example-1.2.3/LICENSE", errors)

    def test_each_repeated_entry_is_audited(self):
        text = self.text + "=" * 57 + "\nexample (1.2.3)\nLicense: MIT\n[LICENSE]\n"
        self.assertIn("Missing source license/attribution text in NOTICE: example-1.2.3/LICENSE", self.audit(text))

    def test_shared_dependency_definition_is_audited(self):
        text = "[See shared dependency notice D001: example (1.2.3).]\n\n"
        text += "SHARED DEPENDENCY NOTICES\nShared dependency notice D001\n\n" + self.text
        self.assertEqual(self.audit(text), [])

    def test_explicit_apache_appendix_can_supply_terms(self):
        terms = "Apache License.\nGrant of Copyright License.\n"
        appendix = "APPENDIX: Copyright Example contributors.\n"
        (self.crate / "LICENSE").write_text(terms + appendix)
        text = "example (1.2.3)\nLicense: Apache-2.0\n"
        text += "[LICENSE: terms in the appendix below]\n" + appendix
        text += "=" * 57 + "\nApache License 2.0 (appendix)\n" + terms
        self.assertEqual(self.audit(text), [])
        self.assertTrue(self.audit(text.replace(appendix, "")))

    def test_declared_file_with_unconventional_name(self):
        (self.crate / "Cargo.toml").write_text(self.manifest + 'license-file = "legal/terms.txt"\n')
        path = self.crate / "legal/terms.txt"
        path.parent.mkdir()
        path.write_text("Additional declared terms.\n")
        self.assertTrue(self.audit())
        self.assertEqual(self.audit(self.text + "Additional declared terms.\n"), [])

    def test_xml_schema_names_are_not_license_files(self):
        for name in ("License_v1.xml", "LicenseCollection_v1.xml"):
            (self.crate / name).write_text("<Schema />")
        self.assertEqual(self.audit(), [])

    def test_empty_and_unavailable_sources_fail(self):
        self.assertTrue(notice.audit_crate_sources(self.text, [self.root / "missing"]))
        self.assertTrue(notice.audit_crate_sources(self.text.replace("1.2.3", "9.0.0"), [self.root]))
        (self.crate / "LICENSE").write_text("")
        self.assertTrue(self.audit())
        (self.crate / "LICENSE").unlink()
        self.assertTrue(self.audit())

    def test_manifest_identity_must_match(self):
        (self.crate / "Cargo.toml").write_text(self.manifest.replace("1.2.3", "9.0.0"))
        self.assertTrue(self.audit())

    def test_path_only_files_require_manual_review(self):
        (self.crate / "LICENSE").write_text("../LICENSE-MIT")
        self.assertTrue(any("contains only a path" in error for error in self.audit()))

    def test_multiple_registries(self):
        second = self.root / "second-registry"
        second.mkdir()
        self.assertEqual(notice.audit_crate_sources(self.text, [second, self.root]), [])
        (second / self.crate.name).mkdir()
        self.assertTrue(any("found 2" in error for error in
                            notice.audit_crate_sources(self.text, [second, self.root])))

    def test_license_path_cannot_escape_crate(self):
        outside = self.root / "outside-license"
        outside.write_text(self.license)
        (self.crate / "LICENSE").unlink()
        (self.crate / "LICENSE").symlink_to(outside)
        self.assertTrue(any("outside its crate" in error for error in self.audit()))
