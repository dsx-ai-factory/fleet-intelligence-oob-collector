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
Exit non-zero, without writing NOTICE, if a crate has no legal text: either its
package ships none and no reviewed fallback exists, or a fallback is stale.
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

FALLBACK_DIR = os.path.join(ROOT, "scripts", "notice-fallbacks")
NV_REDFISH = (
    "nv-redfish-0.22.0.LICENSE",
    "https://github.com/dsx-ai-factory/nv-redfish/blob/v0.22.0/LICENSE",
    # The upstream LICENSE appendix says "Copyright 2020", which predates the
    # project. The source file headers at v0.22.0 carry 2025 and 2026.
    "Copyright (c) 2025-2026 NVIDIA CORPORATION & AFFILIATES. All rights reserved.",
)
TAGGED_TYPES = (
    "tagged-types-0.2.4.LICENSE",
    "https://github.com/poroh/tagged-types/blob/v0.2.4/LICENSE",
    None,
)

# Reviewed legal text for crates whose published package omits it, keyed by
# exact (name, version) so a version bump is unresolved until re-reviewed.
# Each value is (license file, upstream URL, copyright line or None). The
# copyright line is for upstreams whose LICENSE does not state it.
FALLBACKS = {
    **{
        (name, "0.22.0"): NV_REDFISH
        for name in (
            "nv-redfish",
            "nv-redfish-bmc-http",
            "nv-redfish-bmc-mock",
            "nv-redfish-core",
            "nv-redfish-csdl-compiler",
            "nv-redfish-dispatcher",
            "nv-redfish-quirks",
            "nv-redfish-schema",
        )
    },
    ("tagged-types", "0.2.4"): TAGGED_TYPES,
    ("tagged-types-derive", "0.2.4"): TAGGED_TYPES,
    ("valuable", "0.1.1"): (
        "valuable-0.1.1.LICENSE",
        "https://github.com/tokio-rs/valuable/blob/v0.1.1/LICENSE",
        None,
    ),
}

# Files that carry license terms. COPYRIGHT, NOTICE and AUTHORS are attribution
# files and are included in an entry but do not make it resolved on their own.
# A file named by the crate's license-file field also counts as license terms.
TERMS_PREFIXES = ("LICENSE", "LICENCE", "COPYING", "UNLICENSE")
LICENSE_PREFIXES = TERMS_PREFIXES + ("COPYRIGHT", "NOTICE", "AUTHORS")

# Crates whose license terms were reviewed inside a shipped attribution file.
# r-efi 6.0.0 ships the MIT, Apache-2.0 and LGPL-2.1 terms in AUTHORS.
TERMS_IN_ATTRIBUTION = {("r-efi", "6.0.0")}


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


def declared_license_file(pkg, crate_dir):
    """Return the crate's license-file path relative to crate_dir, or None.

    The path comes from the crate manifest, so it is ignored unless it resolves
    to a regular file inside the crate directory.
    """
    declared = pkg.get("license_file")
    if not declared:
        return None
    root = os.path.realpath(crate_dir)
    path = os.path.realpath(os.path.join(crate_dir, declared))
    if not path.startswith(root + os.sep) or not os.path.isfile(path):
        return None
    return os.path.relpath(path, root)


def add_license_text(lines, label, text, apache_text):
    """Append one license file to a crate section, deduping the Apache-2.0 terms."""
    parts = split_apache(text, apache_text)
    if parts:
        lines.append(f"[{label}: Apache License 2.0, terms in the appendix below]")
        lines.extend(t for t in parts if t)
    else:
        lines.append(f"[{label}]")
        lines.append(text)
    lines.append("")


def crate_entry(pkg, apache_text, unresolved):
    """Build the NOTICE section for one crate, recording it in unresolved if it has no legal text."""
    crate_dir = os.path.dirname(pkg["manifest_path"])
    lines = [
        f"{pkg['name']} ({pkg['version']})",
        f"License: {pkg['license'] or 'see crate'}",
        f"URL: https://crates.io/crates/{pkg['name']}",
        "",
    ]
    names = license_files(crate_dir)
    declared = declared_license_file(pkg, crate_dir)
    if declared and declared not in names:
        names.append(declared)
    found = False
    has_terms = False
    for name in names:
        with open(os.path.join(crate_dir, name), encoding="utf-8", errors="replace") as f:
            text = f.read().strip()
        if not text:
            continue
        found = True
        has_terms = has_terms or name == declared or os.path.basename(name).upper().startswith(TERMS_PREFIXES)
        add_license_text(lines, name, text, apache_text)
    key = (pkg["name"], pkg["version"])
    if has_terms or (found and key in TERMS_IN_ATTRIBUTION):
        return "\n".join(lines)
    fallback = FALLBACKS.get(key)
    if not fallback:
        unresolved.append(f"{pkg['name']} {pkg['version']}")
        return "\n".join(lines)
    filename, url, copyright_line = fallback
    with open(os.path.join(FALLBACK_DIR, filename), encoding="utf-8") as f:
        text = f.read().strip()
    lines.append(f"The published crate ships no license file. Upstream license text from {url}:")
    lines.append("")
    if copyright_line:
        lines.append(f"{copyright_line} (from the source file headers of this release)")
        lines.append("")
    add_license_text(lines, "LICENSE", text, apache_text)
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
    unresolved = []
    sections = [HEADER]
    for pkg in pkgs:
        sections.append(crate_entry(pkg, apache_text, unresolved))
    if unresolved:
        sys.exit(
            "No legal text for: "
            + ", ".join(unresolved)
            + ". Review the upstream release and add it to FALLBACKS, or to"
            + " TERMS_IN_ATTRIBUTION if its terms ship in AUTHORS or NOTICE."
        )
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
