#!/usr/bin/env bash
# Copyright (c) 2026, NVIDIA CORPORATION & AFFILIATES. All rights reserved.
# SPDX-License-Identifier: Apache-2.0
set -euo pipefail
work=$(mktemp -d /tmp/rpm-install.XXXXXXXX)
trap 'rm -rf "$work"; rmdir /run/systemd/system 2>/dev/null || true' EXIT
(cd /packages && sha256sum -c checksums.txt)
archive="/packages/$(awk '{print $2}' /packages/checksums.txt)"
# Real RPM transactions execute the packaged scriptlets. Only the systemd
# manager is replaced: this container does not boot systemd as PID 1.
mkdir -p "$work/bin" /run/systemd/system
export RPM_SERVICE_CALLS="$work/services" ACTIVE_SERVICES='' FAIL_RESTART=0
cat > "$work/bin/systemctl" <<'STUB'
#!/bin/sh
set -eu
printf '%s\n' "$*" >> "$RPM_SERVICE_CALLS"
case "$1" in
    is-active) case " $ACTIVE_SERVICES " in *" $3 "*) exit 0;; *) exit 3;; esac ;;
    try-restart) [ "$FAIL_RESTART" = 0 ] ;;
    --no-reload) exec /usr/bin/systemctl.package-test --root=/ "$@" ;;
esac
STUB
chmod 755 "$work/bin/systemctl"
# RPM sets its own scriptlet PATH, so install the manager stub at the real
# executable path inside this disposable container.
mv /usr/bin/systemctl /usr/bin/systemctl.package-test
install -m755 "$work/bin/systemctl" /usr/bin/systemctl
export PATH="$work/bin:$PATH"
: > "$RPM_SERVICE_CALLS"
rpm -ivh "$archive"
if grep -E '^(enable|start|try-restart|restart) ' "$RPM_SERVICE_CALLS"; then
    echo 'Fresh RPM installation started collection' >&2; exit 1
fi
[ "$(id -u fleetint-oob)" -ne 0 ]
[ "$(id -g fleetint-oob)" -ne 0 ]
[ "$(id -gn fleetint-oob)" = fleetint-oob ]
/usr/bin/bmc-collector --help >/dev/null
sh /startup.sh /usr/lib/fleetint-oob/run-service /usr/lib/systemd/system
cmp /expected-NOTICE /usr/share/doc/fleetint-oob/NOTICE
[ -s /usr/share/doc/fleetint-oob/copyright ]
[ "$(rpm -qp --qf '%{VENDOR}' "$archive")" = 'NVIDIA Corporation' ]
for component in collector-dependencies nico-dependencies; do
    [ -s "/usr/share/doc/fleetint-oob/$component/dependencies.txt" ]
    [ -n "$(find "/usr/share/doc/fleetint-oob/$component/licenses" -type f -print -quit)" ]
done
systemd-analyze verify /usr/lib/systemd/system/fleetint-oob-collector.service /usr/lib/systemd/system/fleetint-nico-health.service /usr/lib/systemd/system/fleetint-otel-gateway.service /usr/lib/systemd/system/fleetint-oob.target
conf=/etc/fleetint-oob
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
# The runtime fixture starts all three actual installed daemons and verifies
# token precedence, authenticated gateway export and inventory flush on exit.
bash /runtime.sh
printf '\n# local operator setting\n' >> "$conf/service.conf"
sha256sum "$conf/service.conf" "$conf/config.toml" > "$work/config.sha256"
printf 'outside fixture' > "$work/outside-credential"
chmod 600 "$work/outside-credential"
ln -s "$work/outside-credential" "$conf/credentials/symlink"
chmod 600 "$conf/credentials/credentials.json"
chown root:root "$conf/credentials/credentials.json"
systemd-tmpfiles --create --prefix=/etc/fleetint-oob
[ "$(stat -c %a "$conf/credentials/credentials.json")" = 600 ]
/usr/bin/systemctl.package-test --root=/ --no-reload enable fleetint-oob.target fleetint-oob-collector.service fleetint-nico-health.service fleetint-otel-gateway.service
export ACTIVE_SERVICES=fleetint-oob-collector.service
: > "$RPM_SERVICE_CALLS"
upgrade=$(rpm -qp --qf '/packages/test-fixtures/%{NAME}-%{VERSION}+test1-%{RELEASE}.%{ARCH}.rpm' "$archive")
[ -s "$upgrade" ] || { echo 'Missing matching RPM upgrade fixture' >&2; exit 1; }
rpm -Uvh "$upgrade"
sha256sum -c "$work/config.sha256"
printf 'try-restart fleetint-oob-collector.service\n' > "$work/expected"
grep '^try-restart ' "$RPM_SERVICE_CALLS" > "$work/restarts"
cmp "$work/expected" "$work/restarts"
[ -L /etc/systemd/system/multi-user.target.wants/fleetint-oob-collector.service ]
runuser -u fleetint-oob -- test -r "$conf/credentials/credentials.json"
[ "$(stat -c %a "$work/outside-credential")" = 600 ]
[ "$(stat -c %g "$work/outside-credential")" = 0 ]
# A failed restart must warn without aborting the real package transaction.
export FAIL_RESTART=1
rpm -Uvh --replacepkgs "$upgrade" > "$work/reinstall.log" 2>&1
grep -F '[WARNING] Could not restart fleetint-oob-collector.service' "$work/reinstall.log"
sha256sum -c "$work/config.sha256"
rpm -e fleetint-oob
[ ! -e /usr/bin/bmc-collector ]
[ ! -e /usr/lib/fleetint-oob/nico-hardware-health ]
[ ! -e /etc/systemd/system/multi-user.target.wants/fleetint-oob-collector.service ]
# RPM saves modified package configuration and retains operator credentials.
[ -e "$conf/service.conf.rpmsave" ]
[ -e "$conf/config.toml.rpmsave" ]
[ -e "$conf/credentials/credentials.json" ]
getent passwd fleetint-oob >/dev/null
printf 'RPM installation, runtime, upgrade and removal checks passed\n'
