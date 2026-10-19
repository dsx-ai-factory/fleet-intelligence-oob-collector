#!/bin/sh
# Copyright (c) 2026, NVIDIA CORPORATION & AFFILIATES. All rights reserved.
# SPDX-License-Identifier: Apache-2.0
set -eu

test_root=$(mktemp -d)
trap 'rm -rf "$test_root"' EXIT HUP INT TERM
mkdir -p "$test_root/bin" "$test_root/systemd" "$test_root/etc/fleetint-oob"
touch "$test_root/etc/fleetint-oob/service.conf"
export HOOK_TEST_LOG="$test_root/calls"
export PATH="$test_root/bin:$PATH"
export DPKG_ROOT=

# All service commands are mocked, including helper state queries.
cat > "$test_root/bin/systemctl" <<'EOF'
#!/bin/sh
printf '%s %s\n' "${0##*/}" "$*" >> "$HOOK_TEST_LOG"
case "$*" in
    debian-installed*|--quiet*) exit "${HOOK_TEST_EXISTING:-1}" ;;
esac
exit "${HOOK_TEST_FAILURE:-0}"
EOF
chmod +x "$test_root/bin/systemctl"
cp "$test_root/bin/systemctl" "$test_root/bin/deb-systemd-invoke"
cp "$test_root/bin/systemctl" "$test_root/bin/deb-systemd-helper"

run_hook() {
    : > "$HOOK_TEST_LOG"
    # Redirect only filesystem paths; execute the actual maintainer hook body.
    sed -e "s|/run/systemd/system|$test_root/systemd|g" \
        -e "s|/etc/fleetint-oob/service.conf|$test_root/etc/fleetint-oob/service.conf|g" \
        "deployments/packages/scripts/$1" > "$test_root/hook"
    sh "$test_root/hook" "$2"
}

units='fleetint-oob.target fleetint-oob-collector.service fleetint-nico-health.service fleetint-otel-gateway.service'

run_hook deb/postinst configure
test "$(stat -c %a "$test_root/etc/fleetint-oob/service.conf")" = 600
! grep -Eq ' enable | start |restart' "$HOOK_TEST_LOG"
test "$(grep -c 'update-state' "$HOOK_TEST_LOG")" = 4
grep -q 'systemctl --system daemon-reload' "$HOOK_TEST_LOG"

export HOOK_TEST_EXISTING=0
run_hook deb/postinst configure
test "$(grep -c 'deb-systemd-helper enable' "$HOOK_TEST_LOG")" = 4
unset HOOK_TEST_EXISTING

run_hook deb/prerm upgrade
test ! -s "$HOOK_TEST_LOG"
run_hook deb/prerm remove
grep -Fx "deb-systemd-invoke stop $units" "$HOOK_TEST_LOG"
! grep -q 'disable' "$HOOK_TEST_LOG"
run_hook deb/prerm failed-upgrade
grep -q 'deb-systemd-invoke stop' "$HOOK_TEST_LOG"

run_hook deb/postrm remove
grep -Fx "deb-systemd-helper mask $units" "$HOOK_TEST_LOG"
run_hook deb/postrm purge
grep -Fx "deb-systemd-helper purge $units" "$HOOK_TEST_LOG"
grep -Fx "deb-systemd-helper unmask $units" "$HOOK_TEST_LOG"

export DPKG_ROOT=/unused-test-root
run_hook deb/prerm remove
test ! -s "$HOOK_TEST_LOG"
run_hook deb/postrm remove
! grep -q '^systemctl' "$HOOK_TEST_LOG"
unset DPKG_ROOT

run_hook rpm/postinstall 1
test "$(cat "$HOOK_TEST_LOG")" = 'systemctl daemon-reload'
run_hook rpm/preremove 1
test ! -s "$HOOK_TEST_LOG"
run_hook rpm/preremove 0
grep -Fx "systemctl --no-reload disable $units" "$HOOK_TEST_LOG"
grep -Fx "systemctl stop $units" "$HOOK_TEST_LOG"

# Service-management errors must not leave package transactions broken.
export HOOK_TEST_FAILURE=1
run_hook deb/postinst configure
run_hook deb/prerm remove
run_hook deb/postrm purge
run_hook rpm/postinstall 1
run_hook rpm/preremove 0
run_hook rpm/postremove 0
unset HOOK_TEST_FAILURE

rmdir "$test_root/systemd"
run_hook deb/prerm remove
test ! -s "$HOOK_TEST_LOG"
run_hook deb/postrm remove
grep -q 'deb-systemd-helper mask' "$HOOK_TEST_LOG"
! grep -q '^systemctl' "$HOOK_TEST_LOG"
run_hook rpm/preremove 0
grep -q 'disable' "$HOOK_TEST_LOG"
! grep -q 'systemctl stop' "$HOOK_TEST_LOG"
echo 'Package lifecycle hook checks passed.'
