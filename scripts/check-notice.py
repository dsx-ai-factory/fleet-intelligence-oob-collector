#!/usr/bin/env python3
# Copyright (c) 2026, NVIDIA CORPORATION & AFFILIATES. All rights reserved.
# SPDX-License-Identifier: Apache-2.0
"""Check static attribution coverage markers, shared references, and reviewed inputs."""

import argparse
import hashlib
from pathlib import Path
import re

ROOT = Path(__file__).resolve().parent.parent
INPUT_FILES = (
    "Cargo.toml", "Cargo.lock", "rust-toolchain.toml", "LICENSE", "Makefile",
    "Dockerfile", "Dockerfile.nico-hardware-health", "deployments/packages/Dockerfile",
)
DIGEST_LABEL = "Reviewed attribution inputs SHA256: "
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


def input_digest(root=ROOT):
    # Include build definitions as well as manifests: feature flags/toolchains can
    # change bundled code without changing a dependency version in Cargo.lock.
    paths = sorted(set(INPUT_FILES) | {
        path.relative_to(root).as_posix() for path in (root / "crates").rglob("Cargo.toml")
    })
    digest = hashlib.sha256()
    for name in paths:
        digest.update(name.encode() + b"\0" + (root / name).read_bytes() + b"\0")
    for pin in source_pins(root):
        digest.update(pin.encode() + b"\0")
    return digest.hexdigest()


def check_notice(text, root=ROOT):
    errors = []
    recorded = re.findall(r"^" + DIGEST_LABEL + r"([a-f0-9]{64})$", text, re.M)
    if recorded != [input_digest(root)]:
        errors.append("NOTICE attribution inputs changed: review dependency/source-pin changes, "
                      "update attribution, then record --print-input-digest after review")
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
    if len(references) != len(re.findall(r"\[See shared [^\]\n]*\]", text)):
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
    return errors


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--print-input-digest", action="store_true",
                        help="Print the current dependency/build-input digest for a reviewed NOTICE update")
    args = parser.parse_args()
    if args.print_input_digest:
        print(DIGEST_LABEL + input_digest())
        return
    errors = check_notice((ROOT / "NOTICE").read_text())
    if errors:
        raise SystemExit("\n".join(errors))
    print("NOTICE dependency inputs, components, and shared references checked.")


if __name__ == "__main__":
    main()
