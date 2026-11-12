#!/usr/bin/env python3
# Copyright (c) 2026, NVIDIA CORPORATION & AFFILIATES. All rights reserved.
# SPDX-License-Identifier: Apache-2.0
"""Check locked collector dependencies, component revisions, and shared NOTICE references."""

import argparse
from pathlib import Path
import re
import tomllib

ROOT = Path(__file__).resolve().parent.parent
COMPONENTS = (
    "Fleet Intelligence OOB Collector", "NICo hardware health (carbide-health)",
    "Fleet Intelligence OpenTelemetry gateway", "Go runtime and standard library",
    "SHARED DEPENDENCY NOTICES", "SHARED LICENSE TEXTS",
)


def source_pins(root):
    nico = re.findall(r"^  NICO_SOURCE_REF: ([a-f0-9]{40})$",
                      (root / ".github/workflows/release.yml").read_text(), re.M)
    agent = re.findall(r"^AGENT_REF \?= ([a-f0-9]{40})$", (root / "Makefile").read_text(), re.M)
    if len(nico) != 1 or len(agent) != 1:
        raise ValueError("Expected exactly one pinned NICo and agent source revision")
    return nico[0], agent[0]


def dependency_entries(text, definitions, errors):
    headers = list(re.finditer(
        r"^([-A-Za-z0-9_]+) \(([0-9][^\n]*)\)\nLicense: ([^\n]+)(?:\n|$)", text, re.M))
    boundary = re.compile(
        r"^(?:" + "|".join(re.escape(heading) for heading in COMPONENTS)
        + r"|Shared (?:dependency notice|license text) [A-Z][0-9]+"
        + r"|Apache License 2\.0 \(appendix\)|BUNDLED COMPONENT NOTICES)$"
        + r"|^\[See shared dependency notice ", re.M)
    # Recognize inline license grants, rather than metadata, copyright lines,
    # file labels, or paths standing in for license text.
    grant = re.compile(
        r"Permission\s+(?:is\s+(?:hereby\s+)?granted|to\s+use)"
        r"|Redistribution\s+and\s+use|Grant\s+of\s+Copyright\s+License"
        r"|Data Recipient may (?:use|share)|This is free and unencumbered"
        r"|CC0.*?waiv|GNU.*?TERMS AND CONDITIONS|Mozilla Public License.*?Definitions",
        re.I | re.S)
    def shared_text(body):
        return any(grant.search(definitions.get(("license text", identifier), ""))
                   for identifier in re.findall(r"\[See shared license text ([AM][0-9]+) below\.\]", body))

    appendix_available = False
    for appendix in re.finditer(r"^Apache License 2\.0 \(appendix\)$", text, re.M):
        stop = boundary.search(text, appendix.end())
        appendix_available |= shared_text(text[appendix.end():stop.start() if stop else len(text)])
    entries = set()
    for index, header in enumerate(headers):
        end = headers[index + 1].start() if index + 1 < len(headers) else len(text)
        stop = boundary.search(text, header.end(), end)
        body = text[header.end():stop.start() if stop else end]
        appendix = ("Apache-2.0" in header[3]
                    and re.search(r"terms in (?:the )?appendix below", body)
                    and appendix_available)
        if shared_text(body) or appendix or grant.search(body):
            entries.add((header[1], header[2]))
        else:
            errors.append(f"Missing license text or shared reference in NOTICE entry: {header[1]} ({header[2]})")
    return entries


def check_notice(text, root=ROOT):
    errors = []
    with (root / "Cargo.lock").open("rb") as lockfile:
        locked = {(package["name"], package["version"])
                  for package in tomllib.load(lockfile)["package"] if package.get("source")}
    positions = []
    for heading in COMPONENTS:
        matches = list(re.finditer(r"^" + re.escape(heading) + r"$", text, re.M))
        if len(matches) != 1:
            errors.append(f"Expected exactly one NOTICE section: {heading}")
        else:
            positions.append(matches[0].start())
    if len(positions) != len(COMPONENTS):
        return errors
    if positions != sorted(positions):
        return errors + ["NOTICE component/shared sections are out of order"]
    sections = [text[start:end] for start, end in zip(positions, positions[1:] + [len(text)])]
    for index in (0, 1):
        if not re.search(r"^License: |\[See shared dependency notice D", sections[index], re.M):
            errors.append(f"Missing dependency notices in {COMPONENTS[index]}")
    for section, pin in zip(sections[1:3], source_pins(root)):
        if f"Revision: {pin}\n" not in section:
            errors.append("NOTICE component revision does not match its source pin")
    if "THIRD PARTY LICENSES AND NOTICES" not in sections[2] or "Copyright" not in sections[2]:
        errors.append("Missing gateway license/attribution content")
    if "Copyright" not in sections[3] or "Redistribution and use" not in sections[3]:
        errors.append("Missing Go standard-library license content")

    definitions = {}
    for kind, section in (("dependency notice", sections[4]), ("license text", sections[5])):
        matches = list(re.finditer(r"^Shared " + kind + r" ([ADM][0-9]+)\n\n", section, re.M))
        for index, match in enumerate(matches):
            key = (kind, match[1])
            body = section[match.end():matches[index + 1].start() if index + 1 < len(matches) else len(section)]
            if key in definitions:
                errors.append(f"Duplicate shared definition: {key}")
            if not body.strip().strip("=").strip():
                errors.append(f"Empty shared definition: {key}")
            if kind == "license text" and "[See shared " in body:
                errors.append(f"Shared license definitions must contain text, not further references: {key}")
            definitions[key] = body
    references = re.findall(r"\[See shared (license text|dependency notice) ([A-Za-z][0-9]+)([^\]\n]*)\]", text)
    # Count marker starts independently: incomplete pointers must not disappear
    # from both sides of the syntax check when a closing bracket is missing.
    if len(references) != text.count("[See shared"):
        errors.append("Malformed shared notice/license reference")
    if "[See shared dependency notice " in sections[4]:
        errors.append("Shared dependency definitions must not reference other dependency definitions")
    for kind, identifier, suffix in references:
        key = (kind, identifier)
        if key not in definitions:
            errors.append(f"Broken shared reference: {key}")
        elif kind == "dependency notice":
            expected = ": " + definitions[key].splitlines()[0] + "."
            if suffix != expected:
                errors.append(f"Dependency reference label does not match {identifier}")
        elif suffix != " below.":
            errors.append(f"Malformed license reference: {identifier}")
    for key in definitions.keys() - {(kind, identifier) for kind, identifier, _ in references}:
        errors.append(f"Unreferenced shared definition: {key}")
    if not references or not definitions:
        errors.append("Missing shared notice/license references or definitions")
    entries = dependency_entries(text, definitions, errors)
    for name, version in sorted(locked - entries):
        errors.append(f"Missing NOTICE entry for locked dependency: {name} ({version})")
    return errors


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.parse_args()
    errors = check_notice((ROOT / "NOTICE").read_text())
    if errors:
        raise SystemExit("\n".join(errors))
    print("NOTICE locked dependencies, components, and shared references checked.")


if __name__ == "__main__":
    main()
