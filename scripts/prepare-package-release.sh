#!/usr/bin/env bash
# Copyright (c) 2026, NVIDIA CORPORATION & AFFILIATES. All rights reserved.
# SPDX-License-Identifier: Apache-2.0
set -euo pipefail
[[ $# == 2 ]] || { echo "Usage: $0 <dist-directory> <version>" >&2; exit 1; }
dist_dir="$(cd "$1" && pwd)"
version="$2"
[[ "$version" =~ ^[0-9]+\.[0-9]+\.[0-9]+(-[0-9A-Za-z.-]+)?$ ]] || { echo "Invalid release version" >&2; exit 1; }
shopt -s nullglob
packages=("$dist_dir"/*.deb)
[[ ${#packages[@]} == 2 ]] || { echo "Expected exactly two architecture packages" >&2; exit 1; }
for arch in amd64 arm64; do
  [[ -s "$dist_dir/fleetint-oob_${version}_${arch}.deb" ]] || {
    echo "Missing release package for $arch" >&2; exit 1;
  }
done
rpms=("$dist_dir"/*.rpm)
if [[ ${REQUIRE_RPM:-0} == 1 ]] || (( ${#rpms[@]} )); then
  [[ ${#rpms[@]} == 2 ]] || { echo "Expected exactly two RPM architecture packages" >&2; exit 1; }
  for arch in amd64 arm64; do
    [[ -s "$dist_dir/fleetint-oob_${version}_${arch}.rpm" ]] || {
      echo "Missing release RPM for $arch" >&2; exit 1;
    }
  done
fi
key=deployments/packages/keys/fleet-intelligence.pub.asc
packages+=("${rpms[@]}")
scripts/sign-linux-packages.sh "$dist_dir" "$key"
install -m 0644 "$key" "$dist_dir/fleet-intelligence.pub.asc"
install -m 0755 scripts/verify-linux-package-signature.sh "$dist_dir/verify-linux-package-signature.sh"
# Hash the verified signed packages and the exact supporting release files.
(cd "$dist_dir" && sha256sum "${packages[@]##*/}" \
  fleet-intelligence.pub.asc verify-linux-package-signature.sh > checksums.txt)
scripts/sign-checksum-manifest.sh "$dist_dir/checksums.txt" "$key"
scripts/verify-package-release.sh "$dist_dir"
