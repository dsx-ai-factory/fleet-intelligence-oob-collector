#!/usr/bin/env bash
# Copyright (c) 2026, NVIDIA CORPORATION & AFFILIATES. All rights reserved.
# SPDX-License-Identifier: Apache-2.0
set -euo pipefail
source_dir=$PWD
root=$(mktemp -d /tmp/oob-signing-tests.XXXXXX)
trap 'rm -rf "$root"' EXIT
export GNUPGHOME="$root/gnupg"
install -d -m700 "$GNUPGHOME"
# Check the production identity before substituting the ephemeral test key.
production_pin=$(sed -n 's/^readonly EXPECTED_FINGERPRINT="\([0-9A-F]*\)"$/\1/p' scripts/verify-linux-package-signature.sh)
production_fingerprint=$(gpg --batch --show-keys --with-colons deployments/packages/keys/fleet-intelligence.pub.asc | awk -F: '$1 == "fpr" {print $10; exit}')
[[ -n "$production_pin" && "$production_fingerprint" == "$production_pin" ]] || {
    echo 'Committed production key does not match the verifier fingerprint' >&2; exit 1;
}
gpg --batch --passphrase '' --quick-gen-key 'OOB release test <release@example.test>' ed25519 sign 0
fingerprint=$(gpg --batch --with-colons --list-keys | awk -F: '$1 == "fpr" { print $10; exit }')
project="$root/project"
mkdir -p "$project/scripts" "$project/deployments/packages/keys" "$root/bin" "$project/dist"
cp scripts/*.sh "$project/scripts/"
# Production scripts retain their approved pin. Only temporary copies use this key.
sed -i -e "s/FE0C8B74CA66357C13BE197DCCE3C963087199B3/$fingerprint/g" \
    -e "s/087199b3/${fingerprint: -8}/g" "$project"/scripts/*.sh
export SIGN_TEST_FINGERPRINT="$fingerprint"
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
mkdir -p "$root/rpmbuild/SPECS"
cat > "$root/rpmbuild/SPECS/fixture.spec" <<'SPEC'
Name: fleetint-oob
Version: 1.2.3
Release: 1
Summary: Offline release fixture
License: Apache-2.0
%description
Offline release fixture
%install
mkdir -p %{buildroot}/usr/share/fleetint-oob
printf fixture > %{buildroot}/usr/share/fleetint-oob/payload
%files
/usr/share/fleetint-oob/payload
SPEC
for pair in amd64:x86_64 arm64:aarch64; do
    arch=${pair%:*}
    rpm_arch=${pair#*:}
    rpmbuild -bb --target "$rpm_arch" --define "_topdir $root/rpmbuild" "$root/rpmbuild/SPECS/fixture.spec"
    cp "$root/rpmbuild/RPMS/$rpm_arch/"*.rpm "$dist/fleetint-oob_1.2.3_$arch.rpm"
    cp "$dist/fleetint-oob_1.2.3_$arch.rpm" "$root/original/"
done
: > "$SIGN_LOG"
scripts/prepare-package-release.sh "$dist" 1.2.3
[ "$(wc -l < "$SIGN_LOG")" -eq 5 ]
scripts/verify-package-release.sh "$dist"
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
# Missing both RPMs must fail before any signing request.
rm "$dist/"*.rpm
reject scripts/verify-package-release.sh "$dist"
sign_count=$(wc -l < "$SIGN_LOG")
reject scripts/prepare-package-release.sh "$dist" 1.2.3
[ "$(wc -l < "$SIGN_LOG")" -eq "$sign_count" ]
restore
for arch in amd64 arm64; do
    rpm_package="$dist/fleetint-oob_1.2.3_$arch.rpm"
    mv "$rpm_package" "$root/held.rpm"
    reject scripts/verify-package-release.sh "$dist"
    mv "$root/held.rpm" "$rpm_package"
done
rpm_package="$dist/fleetint-oob_1.2.3_amd64.rpm"
for mode in empty missing; do
    cp "$root/original/${rpm_package##*/}" "$rpm_package"
    export SIGN_MODE=$mode
    reject scripts/sign-linux-packages.sh "$dist" "$key"
    cmp "$root/original/${rpm_package##*/}" "$rpm_package"
    unset SIGN_MODE
    restore
done
truncate -s -1 "$rpm_package"
reject scripts/verify-linux-package-signature.sh "$rpm_package" "$key"
restore
# A correctly signed manifest cannot hide an architecture/version mismatch.
mv "$dist/fleetint-oob_1.2.3_amd64.rpm" "$dist/fleetint-oob_1.2.4_amd64.rpm"
sed -i 's/fleetint-oob_1.2.3_amd64.rpm/fleetint-oob_1.2.4_amd64.rpm/' "$dist/checksums.txt"
scripts/sign-checksum-manifest.sh "$dist/checksums.txt" "$key"
reject scripts/verify-package-release.sh "$dist"
restore
mv "$dist/fleetint-oob_1.2.3_amd64.rpm" "$root/swapped.rpm"
mv "$dist/fleetint-oob_1.2.3_arm64.rpm" "$dist/fleetint-oob_1.2.3_amd64.rpm"
mv "$root/swapped.rpm" "$dist/fleetint-oob_1.2.3_arm64.rpm"
(cd "$dist" && sha256sum fleetint-oob_*.deb fleetint-oob_*.rpm \
    fleet-intelligence.pub.asc verify-linux-package-signature.sh > checksums.txt)
scripts/sign-checksum-manifest.sh "$dist/checksums.txt" "$key"
reject scripts/verify-package-release.sh "$dist"
restore
package="$dist/fleetint-oob_1.2.3_amd64.deb"
for mode in empty missing; do
    cp "$root/original/${package##*/}" "$package"
    export SIGN_MODE=$mode
    reject scripts/sign-linux-packages.sh "$dist" "$key"
    cmp "$root/original/${package##*/}" "$package"
    unset SIGN_MODE
    restore
done
reject scripts/verify-linux-package-signature.sh "$root/original/fleetint-oob_1.2.3_amd64.rpm" "$key"
reject scripts/verify-linux-package-signature.sh "$package" "$source_dir/deployments/packages/keys/fleet-intelligence.pub.asc"
printf unsigned > "$root/extra"
ar q "$package" "$root/extra"
reject scripts/verify-linux-package-signature.sh "$package" "$key"
restore
member=$(ar t "$package" | grep '^data.tar')
printf 'altered signed payload' > "$root/$member"
ar r "$package" "$root/$member"
reject scripts/verify-package-release.sh "$dist"
restore
printf '%064d  fake.deb\n' 0 > "$dist/checksums.txt"
reject scripts/verify-package-release.sh "$dist"
restore
# RC package metadata uses '~' while downloadable names retain the tag.
rc_dist="$root/prerelease"
mkdir -p "$rc_dist"
sed 's/^Version: .*/Version: 1.2.3~rc.1/' "$root/rpmbuild/SPECS/fixture.spec" > "$root/rpmbuild/SPECS/prerelease.spec"
for pair in amd64:x86_64 arm64:aarch64; do
    arch=${pair%:*}
    rpm_arch=${pair#*:}
    sed -i 's/^Version: .*/Version: 1.2.3~rc.1/' "$root/$arch/DEBIAN/control"
    dpkg-deb --build --root-owner-group "$root/$arch" "$rc_dist/fleetint-oob_1.2.3-rc.1_$arch.deb"
    rpmbuild -bb --target "$rpm_arch" --define "_topdir $root/rpmbuild" "$root/rpmbuild/SPECS/prerelease.spec"
    cp "$root/rpmbuild/RPMS/$rpm_arch/fleetint-oob-1.2.3~rc.1-1.$rpm_arch.rpm" "$rc_dist/fleetint-oob_1.2.3-rc.1_$arch.rpm"
done
scripts/prepare-package-release.sh "$rc_dist" 1.2.3-rc.1
scripts/verify-package-release.sh "$rc_dist"
printf 'Offline signing and tamper rejection checks passed\n'
