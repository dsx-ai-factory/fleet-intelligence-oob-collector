#!/usr/bin/env python3
# Copyright (c) 2026, NVIDIA CORPORATION & AFFILIATES. All rights reserved.
# SPDX-License-Identifier: Apache-2.0

"""Validate file links against both source and release-staged Fern pages offline."""

from functools import cache
import importlib.util
import posixpath
import re
from pathlib import Path
from urllib.parse import unquote, urlsplit

ROOT = Path(__file__).resolve().parent.parent
REPO_URL = "https://github.com/dsx-ai-factory/fleet-intelligence-oob-collector"


@cache
def load_stager():
    """Load the release stager and its shared Markdown fence handling."""
    spec = importlib.util.spec_from_file_location(
        "docs_stager", ROOT / "scripts/docs-fetch-versions.py"
    )
    module = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(module)
    return module


def markdown_links(content):
    """Extract inline and reference-style links outside fenced code examples."""
    content = "\n".join(
        "" if in_code_block else line
        for line, in_code_block in load_stager().markdown_lines(content)
    )
    return re.findall(r"\]\(([^\s)]+)(?:\s+\"[^\"]*\")?\)", content) + re.findall(
        r"^\s*\[[^\]]+\]:\s*(\S+)", content, flags=re.MULTILINE
    )


def check_page(content, source, pages, root=ROOT, ref="main"):
    """Return errors for missing files, unpublished pages, refs, and headings."""
    errors = []
    prefix = f"{REPO_URL}/blob/"
    for target in markdown_links(content):
        url = urlsplit(target)
        if target.startswith(prefix):
            expected_prefix = prefix + ref + "/"
            if not target.startswith(expected_prefix):
                errors.append(f"{source}: repository link must use {ref}: {target}")
                continue
            repo_path = unquote(url.path.removeprefix(urlsplit(expected_prefix).path))
            if not (root / repo_path).exists():
                errors.append(f"{source}: missing repository file: {target}")
        elif url.scheme or url.netloc:
            continue
        else:
            destination = (posixpath.normpath(posixpath.join(
                posixpath.dirname(source), unquote(url.path)
            )) if url.path else source)
            if destination not in pages:
                errors.append(
                    f"{source}: not a published page: {target}; use a GitHub blob/main URL"
                )
                continue
            if url.fragment:
                headings = re.findall(r"^#{1,6}\s+(.+)$", pages[destination], re.MULTILINE)
                anchors = [re.sub(r"[^\w -]", "", h.lower()).replace(" ", "-")
                           for h in headings]
                if unquote(url.fragment) not in anchors:
                    errors.append(f"{source}: missing page heading: {target}")
    return errors


def main():
    """Validate published source pages and their release-staged equivalents."""
    # The HEAD navigation is the publication boundary, not the repository tree.
    nav = (ROOT / "fern/docs.yml").read_text()
    paths = re.findall(r"^\s+path:\s+([^\s]+\.md)\s*$", nav, re.MULTILINE)
    if not paths:
        raise SystemExit("No HEAD Markdown pages found in fern/docs.yml")
    pages = {
        (ROOT / "fern" / path).resolve().relative_to(ROOT).as_posix():
        (ROOT / "fern" / path).read_text()
        for path in paths
    }
    stager = load_stager()
    staged_names = dict(stager.ROOT_PAGES)
    # A sentinel ref verifies rewriting without creating or publishing a tag.
    ref = "v0.0.0-link-check"
    staged = {
        staged_names.get(path, path.removeprefix("docs/")): stager.fix_mdx(content, ref)
        for path, content in pages.items()
    }
    errors = []
    for path, content in pages.items():
        errors.extend(check_page(content, path, pages))
    for path, content in staged.items():
        errors.extend(check_page(content, path, staged, ref=ref))
    if errors:
        raise SystemExit("\n".join(errors))
    print(f"Checked {len(pages)} published pages in source and release-staged layouts; "
          "repository file links exist and pin to the release ref.")


if __name__ == "__main__":
    main()
