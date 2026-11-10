#!/usr/bin/env bash
# Copyright (c) 2026, NVIDIA CORPORATION & AFFILIATES. All rights reserved.
# SPDX-License-Identifier: Apache-2.0
set -euo pipefail
source_dir=$PWD
root=$(mktemp -d /tmp/oob-signing-tests.XXXXXX)
trap 'rm -rf "$root"' EXIT
export GNUPGHOME="$root/gnupg"
install -d -m700 "$GNUPGHOME"
gpg --batch --passphrase '' --quick-gen-key 'OOB release test <release@example.test>' ed25519 sign 0
fingerprint=$(gpg --batch --with-colons --list-keys | awk -F: '$1 == "fpr" { print $10; exit }')
project="$root/project"
mkdir -p "$project/scripts" "$project/deployments/packages/keys" "$root/bin" "$project/dist"
cp scripts/*.sh "$project/scripts/"
# Production scripts retain their approved pin. Only temporary copies use this key.
sed -i "s/FE0C8B74CA66357C13BE197DCCE3C963087199B3/$fingerprint/g" "$project"/scripts/*.sh
gpg --batch --armor --export "$fingerprint" > "$project/deployments/packages/keys/fleet-intelligence.pub.asc"
cp deployments/packages/tests/fixtures/nvsec.sh "$root/bin/nvsec"
chmod 755 "$root/bin/"*
export PATH="$root/bin:$PATH" NVSEC_BIN="$root/bin/nvsec"
export NVSEC_SSA_CLIENT_ID=test-client NVSEC_SSA_CLIENT_SECRET=test-client-secret
export SIGN_LOG="$root/sign-log"
: > "$SIGN_LOG"
cd "$project"
dist="$project/dist"
key="$project/deployments/packages/keys/fleet-intelligence.pub.asc"
for arch in amd64 arm64; do
    tree="$root/$arch"
    mkdir -p "$tree/DEBIAN"
    printf 'Package: fleetint-oob\nVersion: 1.2.3\nArchitecture: %s\nMaintainer: Test <test@example.test>\nDescription: offline signing fixture\n' "$arch" > "$tree/DEBIAN/control"
    printf fixture > "$tree/payload"
    dpkg-deb --build --root-owner-group "$tree" "$dist/fleetint-oob_1.2.3_$arch.deb"
done
cp -r "$dist" "$root/original"
scripts/prepare-debian-release.sh "$dist" 1.2.3
[ "$(wc -l < "$SIGN_LOG")" -eq 3 ]
scripts/verify-debian-release.sh "$dist"
cp -r "$dist" "$root/baseline"
restore() {
    rm -f "$dist/"*
    cp "$root/baseline/"* "$dist/"
}
reject() {
    if "$@"; then
        printf 'Unexpected success: %s\n' "$*" >&2
        exit 1
    fi
}
package="$dist/fleetint-oob_1.2.3_amd64.deb"
for mode in empty missing; do
    cp "$root/original/${package##*/}" "$package"
    export SIGN_MODE=$mode
    reject scripts/sign-linux-packages.sh "$dist" "$key"
    cmp "$root/original/${package##*/}" "$package"
    unset SIGN_MODE
    restore
done
reject scripts/verify-linux-package-signature.sh "$package" "$source_dir/deployments/packages/keys/fleet-intelligence.pub.asc"
printf unsigned > "$root/extra"
ar q "$package" "$root/extra"
reject scripts/verify-linux-package-signature.sh "$package" "$key"
restore
member=$(ar t "$package" | grep '^data.tar')
printf 'altered signed payload' > "$root/$member"
ar r "$package" "$root/$member"
reject scripts/verify-debian-release.sh "$dist"
restore
printf '%064d  fake.deb\n' 0 > "$dist/checksums.txt"
reject scripts/verify-debian-release.sh "$dist"
restore
printf 'Offline signing and tamper rejection checks passed\n'
