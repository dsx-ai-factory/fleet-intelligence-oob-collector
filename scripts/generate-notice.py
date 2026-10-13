#!/usr/bin/env python3
# Copyright (c) 2025-2026, NVIDIA CORPORATION & AFFILIATES. All rights reserved.
#
# Licensed under the Apache License, Version 2.0 (the "License");
# you may not use this file except in compliance with the License.
# You may obtain a copy of the License at
#
#     http://www.apache.org/licenses/LICENSE-2.0
#
# Unless required by applicable law or agreed to in writing, software
# distributed under the License is distributed on an "AS IS" BASIS,
# WITHOUT WARRANTIES OR CONDITIONS OF ANY KIND, either express or implied.
# See the License for the specific language governing permissions and
# limitations under the License.
#
# SPDX-License-Identifier: Apache-2.0

"""Generate NOTICE from Cargo.lock and the license files shipped in each crate.

Usage: scripts/generate-notice.py [--check]

Run `cargo fetch --locked` first so crate sources are in the cargo registry.
With --check, exit non-zero if NOTICE is out of date instead of rewriting it.
"""

import json
import os
import subprocess
import sys

ROOT = os.path.dirname(os.path.dirname(os.path.abspath(__file__)))
NOTICE = os.path.join(ROOT, "NOTICE")
SEPARATOR = "=" * 57

HEADER = """Fleet Intelligence OOB Collector
Copyright (c) 2025-2026, NVIDIA CORPORATION & AFFILIATES. All rights reserved.

This product includes software developed by third parties.
The following sets forth attribution notices for third-party
software that may be contained in this product.
"""

LICENSE_PREFIXES = ("LICENSE", "LICENCE", "COPYING", "COPYRIGHT", "UNLICENSE", "NOTICE", "AUTHORS")


def normalize(text):
    """Collapse whitespace so equal license texts compare equal."""
    return " ".join(text.split())


APACHE_START = "Apache License"
APACHE_END = "END OF TERMS AND CONDITIONS"


def split_apache(text, apache_text):
    """Split an Apache-2.0 file into (before, after) the standard terms, or None.

    The terms are kept once in the appendix. Text before them and the appendix
    section after them (where crates put their own copyright line) stay per crate.
    The first Apache-2.0 file seen is the canonical terms. A file whose terms
    differ from it is not split and is embedded in full.
    """
    start = text.find(APACHE_START)
    end = text.find(APACHE_END)
    if start < 0 or end < start or "Version 2.0, January 2004" not in text[start:end]:
        return None
    terms = text[start : end + len(APACHE_END)]
    if normalize(apache_text.setdefault("text", terms)) != normalize(terms):
        return None
    return text[:start].strip(), text[end + len(APACHE_END) :].strip()


def license_files(crate_dir):
    """Return the license, copyright, notice and authors files in a crate directory."""
    names = sorted(
        n
        for n in os.listdir(crate_dir)
        if n.upper().startswith(LICENSE_PREFIXES) and os.path.isfile(os.path.join(crate_dir, n))
    )
    return names


def crate_entry(pkg, apache_text):
    """Build the NOTICE section for one crate."""
    crate_dir = os.path.dirname(pkg["manifest_path"])
    lines = [
        f"{pkg['name']} ({pkg['version']})",
        f"License: {pkg['license'] or 'see crate'}",
        f"URL: https://crates.io/crates/{pkg['name']}",
        "",
    ]
    found = False
    for name in license_files(crate_dir):
        with open(os.path.join(crate_dir, name), encoding="utf-8", errors="replace") as f:
            text = f.read().strip()
        if not text:
            continue
        found = True
        parts = split_apache(text, apache_text)
        if parts:
            lines.append(f"[{name}: Apache License 2.0, terms in the appendix below]")
            lines.extend(t for t in parts if t)
        else:
            lines.append(f"[{name}]")
            lines.append(text)
        lines.append("")
    if not found:
        authors = ", ".join(pkg["authors"]) or "not declared"
        lines.append(f"No license file is shipped with this crate. Authors: {authors}.")
        lines.append("")
    return "\n".join(lines)


def generate():
    """Return the full NOTICE text for the crates in Cargo.lock."""
    out = subprocess.run(
        ["cargo", "metadata", "--format-version", "1", "--locked"],
        cwd=ROOT,
        check=True,
        capture_output=True,
        text=True,
    ).stdout
    pkgs = [p for p in json.loads(out)["packages"] if p["source"]]
    pkgs.sort(key=lambda p: (p["name"], p["version"]))

    apache_text = {}
    sections = [HEADER]
    for pkg in pkgs:
        sections.append(crate_entry(pkg, apache_text))
    if apache_text:
        sections.append("Apache License 2.0 (appendix)\n\n" + apache_text["text"] + "\n")
    body = ("\n" + SEPARATOR + "\n\n").join(sections)
    return body.rstrip("\n") + "\n\n" + SEPARATOR + "\n"


def main():
    """Rewrite NOTICE, or with --check verify it is current."""
    new = generate()
    if "--check" in sys.argv:
        with open(NOTICE, encoding="utf-8") as f:
            if f.read() != new:
                sys.exit("NOTICE is out of date: run scripts/generate-notice.py")
        return
    with open(NOTICE, "w", encoding="utf-8") as f:
        f.write(new)


if __name__ == "__main__":
    main()
