#!/usr/bin/env bash
# Copyright (c) 2026, NVIDIA CORPORATION & AFFILIATES. All rights reserved.
# SPDX-License-Identifier: Apache-2.0
set -euo pipefail
[[ $# == 1 ]] || { echo "Usage: $0 <signed-dist-directory>" >&2; exit 1; }
dist_dir="$(cd "$1" && pwd)"
work_dir="$(mktemp -d "${RUNNER_TEMP:-/tmp}/fleetint-oob-release-verify.XXXXXX")"
trap 'rm -rf "$work_dir"' EXIT
mkdir -m 0700 "$work_dir/gnupg"
key="$dist_dir/fleet-intelligence.pub.asc"
fingerprint="$(gpg --batch --homedir "$work_dir/gnupg" --show-keys --with-colons "$key" | awk -F: '$1 == "fpr" {print $10; exit}')"
[[ "$fingerprint" == FE0C8B74CA66357C13BE197DCCE3C963087199B3 ]] || { echo "Unexpected release signing identity" >&2; exit 1; }
gpg --batch --homedir "$work_dir/gnupg" --dearmor --output "$work_dir/key.gpg" "$key"
gpgv --status-fd 1 --keyring "$work_dir/key.gpg" "$dist_dir/checksums.txt.asc" "$dist_dir/checksums.txt" > "$work_dir/status" 2>&1
grep -q '^\[GNUPG:\] VALIDSIG FE0C8B74CA66357C13BE197DCCE3C963087199B3 ' "$work_dir/status" || { echo "Unexpected checksum signer" >&2; exit 1; }
# Retain DEB-only verification for releases published before RPM support.
shopt -s nullglob
rpms=("$dist_dir"/*.rpm)
[[ ${#rpms[@]} == 0 || ${#rpms[@]} == 2 ]] || { echo "Expected zero or two RPM packages" >&2; exit 1; }
expected_packages=$((2 + ${#rpms[@]}))
# Signed filenames still need validation before any file is hashed.
awk -v expected="$expected_packages" '
  NF != 2 || length($1) != 64 || $1 ~ /[^a-fA-F0-9]/ || $2 ~ /[\/\\]/ || seen[$2]++ {exit 1}
  $2 ~ /^fleetint-oob_[0-9][a-zA-Z0-9.+~-]*_(amd64|arm64)[.](deb|rpm)$/ {packages++; next}
  $2 == "fleet-intelligence.pub.asc" || $2 == "verify-linux-package-signature.sh" {extras++; next}
  {exit 1}
  END {if (NR != expected + 2 || packages != expected || extras != 2) exit 1}
' "$dist_dir/checksums.txt" || { echo "Invalid release checksum inventory" >&2; exit 1; }
(cd "$dist_dir" && sha256sum -c checksums.txt)
version=
formats=(deb)
(( ${#rpms[@]} == 0 )) || formats+=(rpm)
for format in "${formats[@]}"; do
  packages=("$dist_dir"/*."$format")
  [[ ${#packages[@]} == 2 ]] || { echo "Expected exactly two $format architecture packages" >&2; exit 1; }
  for arch in amd64 arm64; do
    matches=("$dist_dir"/fleetint-oob_*_"$arch"."$format")
    [[ ${#matches[@]} == 1 ]] || { echo "Expected one $arch $format package" >&2; exit 1; }
    name="$(basename "${matches[0]}")"
    current="${name#fleetint-oob_}"
    current="${current%_"${arch}"."$format"}"
    [[ -z "$version" || "$version" == "$current" ]] || { echo "Package versions differ" >&2; exit 1; }
    version="$current"
    awk -v name="$name" '$2 == name {count++} END {exit count != 1}' "$dist_dir/checksums.txt" || { echo "Package missing from signed manifest" >&2; exit 1; }
    "$(dirname "$0")/verify-linux-package-signature.sh" "${matches[0]}" "$key"
    if [[ "$format" == rpm ]]; then
      case "$arch" in amd64) rpm_arch=x86_64 ;; arm64) rpm_arch=aarch64 ;; esac
      metadata="$(rpm -qp --qf '%{NAME} %{VERSION} %{ARCH}' "${matches[0]}")"
      [[ "$metadata" == "fleetint-oob ${current/-/\~} $rpm_arch" ]] || {
        echo "RPM metadata does not match its release name" >&2; exit 1;
      }
    fi
  done
done
