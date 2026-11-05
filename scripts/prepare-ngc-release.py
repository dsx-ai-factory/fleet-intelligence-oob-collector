#!/usr/bin/env python3
# Copyright (c) 2026, NVIDIA CORPORATION & AFFILIATES. All rights reserved.
# SPDX-License-Identifier: Apache-2.0

"""Validate private NGC destinations and prepare a chart for public promotion."""

import argparse
import os
from pathlib import Path
import re
import shutil

ROOT = Path(__file__).resolve().parents[1]
CHART_NAME = "fleet-intelligence-oob-collector"
COMPONENT = r"[a-z0-9]+(?:[._-][a-z0-9]+)*"
NUMBER = r"(?:0|[1-9][0-9]*)"
IDENTIFIER = rf"(?:{NUMBER}|[0-9]*[A-Za-z-][0-9A-Za-z-]*)"
VERSION = rf"v({NUMBER}\.{NUMBER}\.{NUMBER}(?:-{IDENTIFIER}(?:\.{IDENTIFIER})*)?)"


def public_prefix(value):
    """Accept only a public NVIDIA NGC image namespace without a tag."""
    if not re.fullmatch(rf"nvcr\.io/nvidia/{COMPONENT}", value):
        raise ValueError("NGC_PUBLIC_IMAGE_PREFIX must be nvcr.io/nvidia/<namespace>")
    return value


def configuration(environ):
    """Validate all settings before any job can publish artifacts."""
    tag = environ.get("RELEASE_VERSION", "")
    match = re.fullmatch(VERSION, tag)
    if not match:
        raise ValueError("Release version must be vMAJOR.MINOR.PATCH, optionally with a prerelease")
    for key in ("NGC_ORG", "NGC_TEAM"):
        if not re.fullmatch(COMPONENT, environ.get(key, "")):
            raise ValueError(f"{key} must name the provisioned private staging organization/team")
    if environ["NGC_ORG"] == "nvidia":
        raise ValueError("CI must publish to private staging, not the public nvidia organization")
    prefix = public_prefix(environ.get("NGC_PUBLIC_IMAGE_PREFIX", ""))
    staging = f"{environ['NGC_ORG']}/{environ['NGC_TEAM']}"
    return {
        "tag": tag,
        "version": match.group(1),
        "image": f"nvcr.io/{staging}/{CHART_NAME}",
        "nico_image": f"nvcr.io/{staging}/nico-hardware-health",
        "chart_target": f"{staging}/{CHART_NAME}",
        "public_image_prefix": prefix,
    }


def prepare_chart(prefix, destination, root=ROOT):
    """Copy chart sources and set final public image names before packaging."""
    public_prefix(prefix)
    source = root / "deployments/helm" / CHART_NAME
    shutil.copytree(source, destination)
    values_path = destination / "values.yaml"
    values = values_path.read_text()
    for image in (CHART_NAME, "nico-hardware-health"):
        old = f"repository: ghcr.io/dsx-ai-factory/{image}"
        if values.count(old) != 1:
            raise ValueError(f"Expected exactly one default image reference for {image}")
        values = values.replace(old, f"repository: {prefix}/{image}")
    values_path.write_text(values)
    shutil.copyfile(root / "LICENSE", destination / "LICENSE")


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--chart-dir", type=Path)
    args = parser.parse_args()
    try:
        if args.chart_dir:
            prepare_chart(os.environ.get("NGC_PUBLIC_IMAGE_PREFIX", ""), args.chart_dir)
        else:
            settings = configuration(os.environ)
            with open(os.environ["GITHUB_OUTPUT"], "a") as output:
                output.writelines(f"{key}={value}\n" for key, value in settings.items())
    except ValueError as error:
        parser.error(str(error))


if __name__ == "__main__":
    main()
