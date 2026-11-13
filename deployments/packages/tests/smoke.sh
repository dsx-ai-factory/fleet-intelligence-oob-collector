#!/bin/sh
# Copyright (c) 2026, NVIDIA CORPORATION & AFFILIATES. All rights reserved.
# SPDX-License-Identifier: Apache-2.0
set -eu
work=$(mktemp -d /tmp/package-install.XXXXXXXX)
trap 'rm -rf "$work"' EXIT
export DEBIAN_FRONTEND=noninteractive
apt-get update -qq
(cd /packages && sha256sum -c checksums.txt)
archive="/packages/$(awk '{print $2}' /packages/checksums.txt)"
apt-get install -y "$archive"
pkg=fleetint-oob
conf=/etc/fleetint-oob
getent passwd fleetint-oob
[ "$(id -u fleetint-oob)" -ne 0 ]
[ "$(id -g fleetint-oob)" -ne 0 ]
bmc-collector --help >/dev/null
bmc-collector run --help >/dev/null
sh /startup.sh /usr/lib/fleetint-oob/run-service /usr/lib/systemd/system
# An explicitly requested token file must never silently fall back to an env token.
for shell in /bin/sh /bin/bash; do
    for service in collector gateway; do
        status=0
        runuser -u fleetint-oob -- env FLEETINT_ENROLLMENT_TOKEN=test-token \
            FLEETINT_ENROLLMENT_TOKEN_FILE=/etc/fleetint-oob/service.conf \
            "$shell" /usr/lib/fleetint-oob/run-service "$service" || status=$?
        [ "$status" = 2 ]
    done
done
if runuser -u fleetint-oob -- env -u FLEETINT_ENROLLMENT_TOKEN -u FLEETINT_ENROLLMENT_TOKEN_FILE /usr/lib/fleetint-oob/run-service gateway; then exit 1; fi
cmp /expected-NOTICE /usr/share/doc/fleetint-oob/NOTICE
# Execute the real hook against an alternate root without any executable tools.
alternate_root="$work/alternate-root"
mkdir -p "$alternate_root/usr/lib/sysusers.d" "$alternate_root/usr/lib/fleetint-oob" \
    "$alternate_root/usr/lib/systemd/system" "$alternate_root/etc/fleetint-oob"
cp /usr/lib/sysusers.d/fleetint-oob.conf "$alternate_root/usr/lib/sysusers.d/"
cp /usr/lib/fleetint-oob/permissions.conf "$alternate_root/usr/lib/fleetint-oob/"
cp /usr/lib/systemd/system/fleetint-oob-collector.service /usr/lib/systemd/system/fleetint-oob.target \
    /usr/lib/systemd/system/fleetint-nico-health.service /usr/lib/systemd/system/fleetint-otel-gateway.service \
    "$alternate_root/usr/lib/systemd/system/"
cp "$conf/config.toml" "$conf/service.conf" "$alternate_root/etc/fleetint-oob/"
printf '%s\n' 'root:x:0:0:root:/root:/bin/sh' > "$alternate_root/etc/passwd"
printf '%s\n' 'root:x:0:' > "$alternate_root/etc/group"
dpkg-deb --control "$archive" "$work/control"
DPKG_ROOT="$alternate_root" /bin/sh "$work/control/postinst" configure
test ! -e "$alternate_root/usr/bin/id"
sha256sum "$alternate_root/etc/passwd" "$alternate_root/etc/group" > "$work/alternate-accounts.sha256"
DPKG_ROOT="$alternate_root" /bin/sh "$work/control/postinst" configure
sha256sum -c "$work/alternate-accounts.sha256"
[ -s /usr/share/doc/fleetint-oob/copyright ]
[ "$(dpkg-deb -f "$archive" Maintainer)" = "NVIDIA Fleet Intelligence Team" ]
systemd-analyze verify /usr/lib/systemd/system/fleetint-oob-collector.service /usr/lib/systemd/system/fleetint-nico-health.service /usr/lib/systemd/system/fleetint-otel-gateway.service /usr/lib/systemd/system/fleetint-oob.target
for unit in fleetint-oob.target fleetint-oob-collector.service fleetint-nico-health.service fleetint-otel-gateway.service; do
    [ ! -e "/etc/systemd/system/multi-user.target.wants/$unit" ]
done
install -d -m750 -o root -g fleetint-oob "$conf/credentials"
printf '%s\n' '{"profiles":{"bmc":{"username":"test-user","password":"test-password"}}}' > "$conf/credentials/credentials.json"
cat > "$conf/config.toml" <<'CONFIG'
[nico_health]
sensor_fetch_interval_seconds = 120
bmc_request_concurrency = 2
leak_detector_enabled = false
leak_detector_poll_interval_seconds = 180
logs_sse_initial_backoff_seconds = 2
logs_sse_max_backoff_seconds = 60
[credentials]
file = "/etc/fleetint-oob/credentials/credentials.json"
[[targets]]
kind = "compute"
credentials_profile = "bmc"
ip = "127.0.0.1"
mac = "02:00:00:00:00:01"
port = 1
CONFIG
chown root:fleetint-oob "$conf" "$conf/config.toml" "$conf/credentials/credentials.json"
chmod 750 "$conf"
chmod 640 "$conf/config.toml" "$conf/credentials/credentials.json"
install -d -m700 -o fleetint-oob -g fleetint-oob /run/fleetint-nico-health
cp "$conf/config.toml" "$work/valid-config"
printf 'invalid TOML [' > "$conf/config.toml"
if runuser -u fleetint-oob -- env FLEETINT_OTLP_ENDPOINT=http://127.0.0.1:4317 /usr/lib/fleetint-oob/run-nico; then exit 1; fi
[ -z "$(ls -A /run/fleetint-nico-health)" ]
cat "$work/valid-config" > "$conf/config.toml"
chmod 600 "$conf/config.toml" "$conf/credentials/credentials.json"
chown root:root "$conf/config.toml" "$conf/credentials/credentials.json"
printf 'outside fixture' > "$work/outside-credential"
chmod 600 "$work/outside-credential"
ln -s "$work/outside-credential" "$conf/credentials/symlink"
# Package permission rules run at installation, not during boot.
[ ! -e /usr/lib/tmpfiles.d/fleetint-oob.conf ]
systemd-tmpfiles --create --prefix=/etc/fleetint-oob
[ "$(stat -c %a "$conf/credentials/credentials.json")" = 600 ]
[ "$(stat -c %g "$conf/credentials/credentials.json")" = 0 ]
systemd-tmpfiles --create /usr/lib/fleetint-oob/permissions.conf
[ "$(stat -c %a "$work/outside-credential")" = 600 ]
[ "$(stat -c %g "$work/outside-credential")" = 0 ]
runuser -u fleetint-oob -- test -r "$conf/credentials/credentials.json"
/bin/bash /runtime.sh
/bin/sh /lifecycle.sh
# Debian owns service lifecycle, and conffiles survive reinstall and removal.
printf '\n# local operator setting\n' >> /etc/fleetint-oob/service.conf
sha256sum /etc/fleetint-oob/service.conf > "$work/config.sha256"
systemctl enable fleetint-oob.target fleetint-oob-collector.service fleetint-nico-health.service fleetint-otel-gateway.service
apt-get install -y --reinstall "$archive"
sha256sum -c "$work/config.sha256"
# Exercise dpkg's upgrade paths without rebuilding either binary.
dpkg-deb --raw-extract "$archive" "$work/upgrade"
version=$(dpkg-deb -f "$archive" Version)
sed -i "s/^Version: .*/Version: ${version}+test1/" "$work/upgrade/DEBIAN/control"
dpkg-deb --root-owner-group --build "$work/upgrade" "$work/upgrade.deb"
apt-get install -y "$work/upgrade.deb"
sha256sum -c "$work/config.sha256"
[ -L /etc/systemd/system/multi-user.target.wants/fleetint-oob-collector.service ]
[ -L /etc/systemd/system/multi-user.target.wants/fleetint-nico-health.service ]
apt-get remove -y "$pkg"
sha256sum -c "$work/config.sha256"
[ ! -e /usr/lib/fleetint-oob/nico-hardware-health ]
apt-get purge -y "$pkg"
[ ! -e /etc/fleetint-oob/service.conf ]
# Operator-created files and the system account are deliberately retained.
[ ! -e "$conf/config.toml" ]
[ -e "$conf/credentials/credentials.json" ]
getent passwd fleetint-oob >/dev/null
printf 'Debian package lifecycle and NICo runtime checks passed\n'
