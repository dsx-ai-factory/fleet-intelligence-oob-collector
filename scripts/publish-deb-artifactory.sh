#!/usr/bin/env bash
# Copyright (c) 2026, NVIDIA CORPORATION & AFFILIATES. All rights reserved.
# SPDX-License-Identifier: Apache-2.0
set -euo pipefail
[[ $# == 1 ]] || { echo "Usage: $0 <signed-dist-directory>|--check-config" >&2; exit 1; }
mode="$1"
: "${ARTIFACTORY_URL:?Set the OOB Debian repository URL}"
: "${ARTIFACTORY_USER:?Set the repository username}"
: "${ARTIFACTORY_TOKEN:?Set the repository token}"
# Validate the destination before a credential file is written or used.
if [[ "$ARTIFACTORY_URL" != https://* || "$ARTIFACTORY_URL" == *[[:space:][:cntrl:]\\\?\#]* ]]; then
  echo "Artifactory requires an HTTPS URL without credentials, query or fragment" >&2; exit 1
fi
authority="${ARTIFACTORY_URL#https://}"
authority="${authority%%/*}"
# Accept DNS/IPv4 hosts and bracketed IPv6 literals, with an optional valid port.
if [[ "$authority" =~ ^([a-zA-Z0-9.-]+)(:([0-9]{1,5}))?$ ]]; then
  artifactory_host="${BASH_REMATCH[1]}"
  port="${BASH_REMATCH[3]}"
elif [[ "$authority" =~ ^\[([a-fA-F0-9:]+)\](:([0-9]{1,5}))?$ ]]; then
  artifactory_host="${BASH_REMATCH[1]}"
  port="${BASH_REMATCH[3]}"
else
  echo "Invalid Artifactory destination" >&2; exit 1
fi
if [[ -n "$port" ]] && (( 10#$port > 65535 )); then
  echo "Invalid Artifactory destination port" >&2; exit 1
fi
[[ "$ARTIFACTORY_USER" != *[$'\r\n\t ']* && "$ARTIFACTORY_TOKEN" != *[$'\r\n\t ']* ]] || {
  echo "Invalid repository credentials" >&2; exit 1;
}
[[ "$mode" != --check-config ]] || exit 0
dist_dir="$(cd "$mode" && pwd)"
# Verify signatures, complete package coverage and hashes before any upload.
scripts/verify-debian-release.sh "$dist_dir"
work_dir="$(mktemp -d "${RUNNER_TEMP:-/tmp}/fleetint-oob-publish.XXXXXX")"
trap 'rm -rf "$work_dir"' EXIT
shopt -s nullglob
packages=("$dist_dir"/*.deb)
credentials="$work_dir/netrc"
umask 077
printf 'machine %s\nlogin %s\npassword %s\n' "$artifactory_host" "$ARTIFACTORY_USER" "$ARTIFACTORY_TOKEN" > "$credentials"
chmod 0600 "$credentials"
for package in "${packages[@]}"; do
  name="$(basename "$package")"
  architecture="${name##*_}"
  architecture="${architecture%.deb}"
  hash="$(sha256sum "$package" | awk '{print $1}')"
  destination="${ARTIFACTORY_URL%/}/$name;deb.distribution=bookworm;deb.component=main;deb.architecture=$architecture"
  existing_status="$(curl --silent --show-error --head --output /dev/null \
    --dump-header "$work_dir/headers" --write-out '%{http_code}' \
    --proto '=https' --max-redirs 0 --connect-timeout 15 --max-time 60 \
    --netrc-file "$credentials" "$destination")"
  if [[ "$existing_status" == 200 ]]; then
    existing_hash="$(awk 'tolower($1) == "x-checksum-sha256:" {gsub(/\r/, "", $2); print tolower($2)}' "$work_dir/headers")"
    [[ "$existing_hash" == "$hash" ]] || { echo "Refusing to overwrite $name with different content" >&2; exit 1; }
    echo "Verified existing identical $name"
    continue
  fi
  [[ "$existing_status" == 404 ]] || { echo "Could not check repository state for $name (HTTP $existing_status)" >&2; exit 1; }
  status="$(curl --silent --show-error --output /dev/null --write-out '%{http_code}' \
    --proto '=https' --max-redirs 0 --connect-timeout 15 --max-time 600 \
    --speed-limit 1024 --speed-time 60 --netrc-file "$credentials" \
    --header 'If-None-Match: *' --header "X-Checksum-Sha256: $hash" \
    --upload-file "$package" "$destination")"
  [[ "$status" =~ ^2[0-9][0-9]$ ]] || { echo "Upload failed for $name (HTTP $status); no further packages uploaded" >&2; exit 1; }
  echo "Published $name"
done
