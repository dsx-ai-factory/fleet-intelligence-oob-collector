#!/usr/bin/env bash
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
# A valid signature does not make filenames safe. Only these four artifacts
# belong in the signed manifest; reject duplicates and paths before hashing.
awk '
  NF != 2 || length($1) != 64 || $1 ~ /[^a-fA-F0-9]/ || $2 ~ /[\/\\]/ || seen[$2]++ {exit 1}
  $2 ~ /^fleetint-oob_[0-9][a-zA-Z0-9.+~-]*_(amd64|arm64)[.]deb$/ {packages++; next}
  $2 == "fleet-intelligence.pub.asc" || $2 == "verify-linux-package-signature.sh" {extras++; next}
  {exit 1}
  END {if (NR != 4 || packages != 2 || extras != 2) exit 1}
' "$dist_dir/checksums.txt" || { echo "Invalid release checksum inventory" >&2; exit 1; }
(cd "$dist_dir" && sha256sum -c checksums.txt)
shopt -s nullglob
packages=("$dist_dir"/*.deb)
[[ ${#packages[@]} == 2 ]] || { echo "Expected exactly two architecture packages" >&2; exit 1; }
version=
for arch in amd64 arm64; do
  matches=("$dist_dir"/fleetint-oob_*_"$arch".deb)
  [[ ${#matches[@]} == 1 ]] || { echo "Expected one $arch package" >&2; exit 1; }
  name="$(basename "${matches[0]}")"
  current="${name#fleetint-oob_}"
  current="${current%_"${arch}".deb}"
  [[ -z "$version" || "$version" == "$current" ]] || { echo "Architecture versions differ" >&2; exit 1; }
  version="$current"
  awk -v name="$name" '$2 == name {count++} END {exit count != 1}' "$dist_dir/checksums.txt" || { echo "Package missing from signed manifest" >&2; exit 1; }
  "$(dirname "$0")/verify-linux-package-signature.sh" "${matches[0]}" "$key"
done
