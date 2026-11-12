#!/usr/bin/env bash
# Copyright (c) 2026, NVIDIA CORPORATION & AFFILIATES. All rights reserved.
# SPDX-License-Identifier: Apache-2.0
set -euo pipefail
readonly EXPECTED_FINGERPRINT="FE0C8B74CA66357C13BE197DCCE3C963087199B3"
script_dir="$(cd "$(dirname "$0")" && pwd)"

submit_signing_job() {
  local input_file=$1 result_dir=$2 result_name=$3 job_type=$4 scope=$5 description=$6
  local nvsec_bin="${NVSEC_BIN:-nvsec}"
  : "${NVSEC_SSA_CLIENT_ID:?NVSEC_SSA_CLIENT_ID is required}"
  : "${NVSEC_SSA_CLIENT_SECRET:?NVSEC_SSA_CLIENT_SECRET is required}"
  command -v "$nvsec_bin" >/dev/null 2>&1 || { echo "nvsec is required" >&2; exit 1; }
  local args=(3s submit --job_type "$job_type" --description "$description"
    --input_file "$input_file" --download --print_log --auth ssa --scope "$scope"
    --timeout 600 --result_dir "$result_dir" --result_filename "$result_name")
  [[ -z "${NSPECT_ID:-}" ]] || args+=(--nspect_id "$NSPECT_ID")
  echo "Signing $(basename "$input_file") with 3S job $job_type"
  "$nvsec_bin" "${args[@]}"
  [[ -s "$result_dir/$result_name" ]] || {
    echo "3S did not return a signature or signed package for $result_name" >&2
    find "$result_dir" -maxdepth 1 -name 'sign_r*.log' -type f -exec tail -n 100 {} \; >&2
    exit 1
  }
}

# Subshells keep signing scratch directories, variables, and cleanup traps separate.
sign_packages() (
  dist_dir="$(cd "$1" && pwd)"
  public_key="$(cd "$(dirname "$2")" && pwd)/$(basename "$2")"
  job_type="${NVSEC_LINUX_PACKAGE_JOB_TYPE:-LINUX_PACKAGE_FLEET_INTELLIGENCE}"
  scope="${NVSEC_LINUX_PACKAGE_SSA_SCOPE:-SIGNING_LINUX_PACKAGE_FLEET_INTELLIGENCE}"

  [[ -s "$public_key" ]] || { echo "Public key not found or empty: $public_key" >&2; exit 1; }

  shopt -s nullglob
  packages=("$dist_dir"/*.deb "$dist_dir"/*.rpm)
  shopt -u nullglob
  (( ${#packages[@]} > 0 )) || {
    echo "No DEB or RPM packages found in $dist_dir" >&2
    exit 1
  }

  work_dir="$(mktemp -d "${RUNNER_TEMP:-/tmp}/fleetint-package-sign.XXXXXX")"
  trap '
    case "$work_dir" in
      "${RUNNER_TEMP:-/tmp}"/fleetint-package-sign.*) rm -rf "$work_dir" ;;
      *) echo "Refusing to remove unexpected temporary directory: $work_dir" >&2 ;;
    esac
  ' EXIT

  for package in "${packages[@]}"; do
    package_name="$(basename "$package")"
    result_dir="$work_dir/$package_name"
    mkdir -p "$result_dir"

    submit_signing_job "$package" "$result_dir" "$package_name" "$job_type" "$scope" \
      "Fleet Intelligence OOB ${GITHUB_REF_NAME:-snapshot} package signing: $package_name"
    signed_package="$result_dir/$package_name"

    "$script_dir/verify-linux-package-signature.sh" "$signed_package" "$public_key"

    replacement="$dist_dir/.$package_name.signed"
    install -m 0644 "$signed_package" "$replacement"
    mv "$replacement" "$package"
    echo "Replaced $package_name with its verified signed package"
  done

  echo "Signed and verified ${#packages[@]} Linux packages"
)

sign_manifest() (
  checksums_file="$(cd "$(dirname "$1")" && pwd)/$(basename "$1")"
  public_key="$(cd "$(dirname "$2")" && pwd)/$(basename "$2")"
  job_type="${NVSEC_CHECKSUM_JOB_TYPE:-FLEET_INTELLIGENCE}"
  scope="${NVSEC_CHECKSUM_SSA_SCOPE:-SIGNING_FLEET_INTELLIGENCE}"
  signature_name="$(basename "$checksums_file").asc"

  command -v gpg >/dev/null 2>&1 || { echo "gpg is required" >&2; exit 1; }
  command -v gpgv >/dev/null 2>&1 || { echo "gpgv is required" >&2; exit 1; }
  [[ -s "$checksums_file" ]] || { echo "Checksum file not found or empty: $checksums_file" >&2; exit 1; }
  [[ -s "$public_key" ]] || { echo "Public key not found or empty: $public_key" >&2; exit 1; }

  work_dir="$(mktemp -d "${RUNNER_TEMP:-/tmp}/fleetint-checksum-sign.XXXXXX")"
  trap 'rm -rf "$work_dir"' EXIT

  submit_signing_job "$checksums_file" "$work_dir" "$signature_name" "$job_type" "$scope" \
    "Fleet Intelligence ${GITHUB_REF_NAME:-snapshot} checksum manifest"
  signature="$work_dir/$signature_name"

  key_home="$work_dir/key-home"
  keyring="$work_dir/fleet-intelligence.gpg"
  status_file="$work_dir/gpg-status"
  mkdir -m 0700 "$key_home"

  fingerprint="$(
    gpg --batch --homedir "$key_home" --show-keys --with-colons "$public_key" |
      awk -F: '$1 == "fpr" { print $10; exit }'
  )"
  [[ "$fingerprint" == "$EXPECTED_FINGERPRINT" ]] || {
    echo "Unexpected Fleet Intelligence key fingerprint: $fingerprint" >&2
    exit 1
  }

  gpg --batch --homedir "$key_home" --yes --dearmor --output "$keyring" "$public_key"
  gpgv --status-fd 1 --keyring "$keyring" "$signature" "$checksums_file" > "$status_file" 2>&1
  grep -q "^\[GNUPG:\] VALIDSIG $EXPECTED_FINGERPRINT " "$status_file" || {
    echo "Checksum signature was not made by the expected Fleet Intelligence key" >&2
    cat "$status_file" >&2
    exit 1
  }

  install -m 0644 "$signature" "$checksums_file.asc"
  echo "Created verified detached signature $checksums_file.asc"
)

sign_release() (
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
  [[ ${#rpms[@]} == 2 ]] || { echo "Expected exactly two RPM architecture packages" >&2; exit 1; }
  for arch in amd64 arm64; do
    [[ -s "$dist_dir/fleetint-oob_${version}_${arch}.rpm" ]] || {
      echo "Missing release RPM for $arch" >&2; exit 1;
    }
  done
  key="$script_dir/../deployments/packages/keys/fleet-intelligence.pub.asc"
  packages+=("${rpms[@]}")
  sign_packages "$dist_dir" "$key"
  install -m 0644 "$key" "$dist_dir/fleet-intelligence.pub.asc"
  install -m 0755 "$script_dir/verify-linux-package-signature.sh" "$dist_dir/verify-linux-package-signature.sh"
  # Hash the verified signed packages and the exact supporting release files.
  (cd "$dist_dir" && sha256sum "${packages[@]##*/}" \
    fleet-intelligence.pub.asc verify-linux-package-signature.sh > checksums.txt)
  sign_manifest "$dist_dir/checksums.txt" "$key"
  "$script_dir/verify-package-release.sh" "$dist_dir"
)

usage() {
  echo "Usage: $0 release <dist-directory> <version>" >&2
  echo "       $0 packages <dist-directory> <public-key.asc>" >&2
  echo "       $0 manifest <checksums-file> <public-key.asc>" >&2
}
mode=${1:-}
case "$mode" in
  -h|--help) usage; exit 0 ;;
  release|packages|manifest)
    shift
    [[ $# == 2 ]] || { usage; exit 2; }
    case "$mode" in
      release) sign_release "$@" ;;
      packages) sign_packages "$@" ;;
      manifest) sign_manifest "$@" ;;
    esac ;;
  *) usage; exit 2 ;;
esac
