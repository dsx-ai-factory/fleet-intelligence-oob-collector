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

    def test_dependency_and_build_input_changes(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            files = list(notice.INPUT_FILES) + [
                p.relative_to(notice.ROOT).as_posix()
                for p in (notice.ROOT / "crates").rglob("Cargo.toml")
            ] + [".github/workflows/release.yml"]
            for name in files:
                path = root / name
                path.parent.mkdir(parents=True, exist_ok=True)
                path.write_bytes((notice.ROOT / name).read_bytes())
            self.assertEqual(notice.check_notice(TEXT, root), [])
            for name in files:
                with self.subTest(input=name):
                    path = root / name
                    original = path.read_bytes()
                    if name.endswith("release.yml"):
                        changed = original.replace(notice.source_pins(root)[0].encode(), b"1" * 40)
                    else:
                        changed = original + b"\n# dependency/build change\n"
                    path.write_bytes(changed)
                    self.assertTrue(notice.check_notice(TEXT, root))
                    path.write_bytes(original)
            added = root / "crates/new-component/Cargo.toml"
            added.parent.mkdir(parents=True)
            added.write_text('[package]\nname = "new-component"\nversion = "1.0.0"\n')
            self.assertTrue(notice.check_notice(TEXT, root))

    def test_pin_revision_mismatch_survives_digest_refresh(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            for path in list(notice.INPUT_FILES) + [
                p.relative_to(notice.ROOT).as_posix()
                for p in (notice.ROOT / "crates").rglob("Cargo.toml")
            ] + [".github/workflows/release.yml"]:
                target = root / path
                target.parent.mkdir(parents=True, exist_ok=True)
                target.write_bytes((notice.ROOT / path).read_bytes())
            nico, agent = notice.source_pins(root)
            for path, pin in [("Makefile", agent), (".github/workflows/release.yml", nico)]:
                with self.subTest(pin=path):
                    target = root / path
                    original = target.read_text()
                    target.write_text(original.replace(pin, "1" * 40))
                    refreshed = TEXT.replace(notice.input_digest(), notice.input_digest(root))
                    self.assertTrue(any("revision" in error for error in notice.check_notice(refreshed, root)))
                    target.write_text(original)

    def test_missing_or_stale_review_digest(self):
        self.assertTrue(notice.check_notice(TEXT.replace(notice.input_digest(), "0" * 64)))
        self.assertTrue(notice.check_notice(re.sub(r"^Reviewed attribution inputs SHA256: .*\n", "", TEXT)))

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

    def test_recursive_license_reference(self):
        self.assertTrue(notice.check_notice(TEXT.replace(
            "Shared license text A01\n\n", "Shared license text A01\n\n[See shared license text A01 below.]\n", 1)))
