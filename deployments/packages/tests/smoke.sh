#!/bin/sh
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
# An explicitly requested token file must never silently fall back to an env token.
if runuser -u fleetint-oob -- env FLEETINT_AGENT_TOKEN=test-token FLEETINT_AGENT_TOKEN_FILE=/nonexistent/token /usr/lib/fleetint-oob/run-service collector; then exit 1; fi
if runuser -u fleetint-oob -- env -u FLEETINT_AGENT_TOKEN -u FLEETINT_AGENT_TOKEN_FILE /usr/lib/fleetint-oob/run-service gateway; then exit 1; fi
[ -s /usr/share/doc/fleetint-oob/NOTICE ]
[ -s /usr/share/doc/fleetint-oob/copyright ]
[ "$(dpkg-deb -f "$archive" Maintainer)" = "NVIDIA Fleet Intelligence Team" ]
docs=/usr/share/doc/fleetint-oob
for component in collector-dependencies nico-dependencies; do
    [ -n "$(find "$docs/$component" -name 'dependencies.txt*' -type f -size +0c -print -quit)" ]
    [ -n "$(find "$docs/$component/licenses" -type f -print -quit)" ]
done
[ -s "$docs/nico-NOTICES" ]
[ -s "$docs/gateway-NOTICES" ]
systemd-analyze verify /usr/lib/systemd/system/fleetint-oob-collector.service /usr/lib/systemd/system/fleetint-nico-health.service /usr/lib/systemd/system/fleetint-otel-gateway.service /usr/lib/systemd/system/fleetint-oob.target
for unit in fleetint-oob.target fleetint-oob-collector.service fleetint-nico-health.service fleetint-otel-gateway.service; do
    [ ! -e "/etc/systemd/system/multi-user.target.wants/$unit" ]
done
install -d -m750 -o root -g fleetint-oob "$conf/credentials"
printf 'test-user\n' > "$conf/credentials/username"
printf 'test-password\n' > "$conf/credentials/password"
cat > "$conf/config.toml" <<'CONFIG'
[credentials]
username_file = "credentials/username"
password_file = "credentials/password"
[[targets]]
kind = "compute"
ip = "127.0.0.1"
mac = "02:00:00:00:00:01"
port = 1
CONFIG
chown root:fleetint-oob "$conf" "$conf/config.toml" "$conf/credentials/username" "$conf/credentials/password"
chmod 750 "$conf"
chmod 640 "$conf/config.toml" "$conf/credentials/username" "$conf/credentials/password"
install -d -m700 -o fleetint-oob -g fleetint-oob /run/fleetint-nico-health
cp "$conf/config.toml" "$work/valid-config"
printf 'invalid TOML [' > "$conf/config.toml"
if runuser -u fleetint-oob -- env FLEETINT_OTLP_ENDPOINT=http://127.0.0.1:4317 /usr/lib/fleetint-oob/run-nico; then exit 1; fi
[ -z "$(ls -A /run/fleetint-nico-health)" ]
cat "$work/valid-config" > "$conf/config.toml"
chmod 600 "$conf/config.toml" "$conf/credentials/username" "$conf/credentials/password"
chown root:root "$conf/config.toml" "$conf/credentials/username" "$conf/credentials/password"
printf 'outside fixture' > "$work/outside-credential"
chmod 600 "$work/outside-credential"
ln -s "$work/outside-credential" "$conf/credentials/symlink"
systemd-tmpfiles --create /usr/lib/tmpfiles.d/fleetint-oob.conf
[ "$(stat -c %a "$work/outside-credential")" = 600 ]
[ "$(stat -c %g "$work/outside-credential")" = 0 ]
runuser -u fleetint-oob -- test -r "$conf/credentials/password"
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
[ -e "$conf/credentials/password" ]
getent passwd fleetint-oob >/dev/null
printf 'Debian package lifecycle and NICo runtime checks passed\n'
