# Copyright (c) 2026, NVIDIA CORPORATION & AFFILIATES. All rights reserved.
# SPDX-License-Identifier: Apache-2.0

import importlib.util
import os
import subprocess
import sys
import tempfile
import unittest
from unittest.mock import patch
from pathlib import Path

spec = importlib.util.spec_from_file_location(
    "docs_links", Path(__file__).with_name("check-docs-links.py")
)
links = importlib.util.module_from_spec(spec)
spec.loader.exec_module(links)

GIT_ENV = {
    key: value for key, value in os.environ.items()
    if key not in {"GIT_DIR", "GIT_WORK_TREE", "GIT_INDEX_FILE", "GIT_OBJECT_DIRECTORY"}
}


def commit_fixture(root):
    subprocess.run(["git", "init", "-q", str(root)], check=True, env=GIT_ENV)
    subprocess.run(["git", "-C", str(root), "add", "."], check=True, env=GIT_ENV)
    subprocess.run([
        "git", "-C", str(root), "-c", "user.name=Docs Tests",
        "-c", "user.email=tests@example.test", "commit", "-qm", "fixture",
    ], check=True, env=GIT_ENV)


class PublicationLinksTest(unittest.TestCase):
    def test_fixture_git_commands_ignore_inherited_repository_environment(self):
        with tempfile.TemporaryDirectory() as directory:
            caller = Path(directory) / "caller"
            caller.mkdir()
            (caller / "caller.txt").write_text("caller data\n")
            commit_fixture(caller)
            caller_index = (caller / ".git/index").read_bytes()
            caller_head = subprocess.run(
                ["git", "-C", str(caller), "rev-parse", "HEAD"],
                check=True, capture_output=True, text=True, env=GIT_ENV,
            ).stdout
            fixture = Path(directory) / "fixture"
            fixture.mkdir()
            (fixture / "fixture.txt").write_text("fixture data\n")
            hostile_env = os.environ | {
                "GIT_DIR": str(caller / ".git"),
                "GIT_WORK_TREE": str(caller),
                "GIT_INDEX_FILE": str(caller / ".git/index"),
                "GIT_OBJECT_DIRECTORY": str(caller / ".git/objects"),
            }
            # Import with the hostile environment, as a Git hook would, without
            # changing this process's environment or touching the working repo.
            subprocess.run([
                sys.executable, "-c",
                "import runpy, sys; from pathlib import Path; "
                "runpy.run_path(sys.argv[1])['commit_fixture'](Path(sys.argv[2]))",
                str(Path(__file__).resolve()), str(fixture),
            ], check=True, env=hostile_env)
            self.assertTrue((fixture / ".git").is_dir())
            files = subprocess.run(
                ["git", "-C", str(fixture), "ls-files"],
                check=True, capture_output=True, text=True, env=GIT_ENV,
            ).stdout.splitlines()
            self.assertEqual(files, ["fixture.txt"])
            content = subprocess.run(
                ["git", "-C", str(fixture), "show", "HEAD:fixture.txt"],
                check=True, capture_output=True, text=True, env=GIT_ENV,
            ).stdout
            self.assertEqual(content, "fixture data\n")
            self.assertEqual((caller / ".git/index").read_bytes(), caller_index)
            self.assertEqual(subprocess.run(
                ["git", "-C", str(caller), "rev-parse", "HEAD"],
                check=True, capture_output=True, text=True, env=GIT_ENV,
            ).stdout, caller_head)

    def test_existing_repository_file_is_not_a_published_page(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            (root / "credentials.example.json").write_text("{}")
            pages = {"docs/credentials.md": "# Credentials"}
            self.assertTrue(links.check_page(
                "[JSON](../credentials.example.json)", "docs/credentials.md", pages, root
            ))

    def test_host_package_is_in_versioned_docs(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            (root / "docs").mkdir()
            (root / "docs/configuration.md").write_text("# Configuration")
            package_dir = root / "deployments/packages"
            package_dir.mkdir(parents=True)
            content = f"[Configuration]({links.REPO_URL}/blob/main/docs/configuration.md)"
            (package_dir / "README.md").write_text(content)
            commit_fixture(root)
            stager = links.load_stager()
            versions = root / "versions"
            with patch.object(stager, "REPO_ROOT", root), patch.object(stager, "VERSIONS_DIR", versions):
                files = stager.fetch_version("test", "HEAD")
            self.assertIn("host-package.md", files)
            staged = (versions / "test-content/host-package.md").read_text()
            self.assertIn("/blob/HEAD/docs/configuration.md", staged)

    def test_linux_guides_and_navigation_are_staged_together(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            (root / "docs").mkdir()
            for name in ("installation.md", "configuration.md", "host-installation.md", "host-configuration.md"):
                (root / "docs" / name).write_text("# Guide\n")
            legacy = root / "deployments/packages/README.md"
            legacy.parent.mkdir(parents=True)
            legacy.write_text("# Package build instructions\n")
            commit_fixture(root)
            stager = links.load_stager()
            with patch.object(stager, "REPO_ROOT", root), patch.object(stager, "VERSIONS_DIR", root / "versions"):
                files = stager.fetch_version("test", "HEAD")
            self.assertIn("host-installation.md", files)
            self.assertIn("host-configuration.md", files)
            self.assertNotIn("host-package.md", files)
            nav = stager.generate_nav_yml("test", files)
            self.assertEqual(nav.count("skip-slug: true"), 2)
            for name in files:
                self.assertEqual(nav.count(f"path: test-content/{name}"), 1)
                self.assertIn(f"slug: {name.removesuffix('.md')}", nav)

    def test_asset_url_pins_to_release(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            (root / "credentials.example.json").write_text("{}")
            content = f"[JSON]({links.REPO_URL}/blob/main/credentials.example.json)"
            staged = links.load_stager().fix_mdx(content, "v1.0.0")
            self.assertIn("/blob/v1.0.0/", staged)
            self.assertFalse(links.check_page(staged, "credentials.md", {}, root, "v1.0.0"))
            self.assertTrue(links.check_page(content, "credentials.md", {}, root, "v1.0.0"))

    def test_overview_links_resolve_after_flattening(self):
        content = "[Configuration](docs/configuration.md#helm-values)"
        staged = links.load_stager().fix_mdx(content, "v1.0.0")
        pages = {"configuration.md": "# Configuration\n## Helm values"}
        self.assertFalse(links.check_page(staged, "overview.md", pages))
        self.assertTrue(links.check_page(content, "overview.md", pages))

    def test_contributing_links_to_staged_overview(self):
        staged = links.load_stager().fix_mdx("[README](README.md)", "v1.0.0")
        self.assertFalse(links.check_page(
            staged, "contributing.md", {"overview.md": "# Overview"}
        ))

    def test_missing_headings_and_repository_files_fail(self):
        self.assertTrue(links.check_page(
            "[Missing](configuration.md#missing)", "overview.md",
            {"configuration.md": "# Configuration"}
        ))
        self.assertTrue(links.check_page(
            f"[Missing]({links.REPO_URL}/blob/main/does-not-exist.json)", "overview.md", {}
        ))

    def test_code_examples_are_not_rewritten_or_checked_as_links(self):
        content = "```markdown\n[Example](docs/example.md)\n```"
        self.assertEqual(content, links.load_stager().fix_mdx(content, "v1.0.0"))
        self.assertFalse(links.markdown_links(content))

    def test_nested_fences_preserve_examples_and_check_links_afterward(self):
        """Shorter, mismatched, or suffixed fences must not expose example links."""
        for opener, inner, closer in [
            ("````markdown", "```", "````"),
            ("~~~markdown", "```", "~~~"),
            ("```markdown", "~~~", "```"),
            ("~~~markdown", "~~", "~~~"),
            ("````markdown", "````not-a-closer", "````"),
            ("```markdown", None, "`````"),
            ("   ````markdown", "   ```", "   ````"),
            ("~~~markdown", "~~~not-a-closer", "~~~~ \t"),
        ]:
            example = f"{opener}\n[Example](docs/example.md)\n"
            if inner is not None:
                example += f"{inner}\n[Still example](docs/still-example.md)\n"
            example += closer
            content = example + "\n[Outside](docs/configuration.md)"
            with self.subTest(opener=opener, inner=inner, closer=closer):
                self.assertEqual(
                    example + "\n[Outside](configuration.md)",
                    links.load_stager().fix_mdx(content, "v1.0.0"),
                )
                self.assertEqual(["docs/configuration.md"], links.markdown_links(content))

    def test_unclosed_fence_preserves_remaining_content(self):
        """An unclosed fence keeps all remaining lines inside the example."""
        content = "~~~~markdown\n[Example](docs/example.md)\n~~~\n[Example](README.md)"
        self.assertFalse(links.markdown_links(content))
        self.assertEqual(content, links.load_stager().fix_mdx(content, "v1.0.0"))


if __name__ == "__main__":
    unittest.main()
