#!/bin/sh
# Copyright (c) 2026, NVIDIA CORPORATION & AFFILIATES. All rights reserved.
# SPDX-License-Identifier: Apache-2.0
set -eu

hook_script_root=${HOOK_SCRIPT_ROOT:-deployments/packages/scripts}
test_root=$(mktemp -d)
trap 'rm -rf "$test_root"' EXIT HUP INT TERM
mkdir -p "$test_root/bin" "$test_root/systemd" "$test_root/etc/fleetint-oob"
touch "$test_root/etc/fleetint-oob/service.conf"
export HOOK_TEST_LOG="$test_root/calls"
export HOOK_TEST_PERMISSION_LOG="$test_root/permissions"
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

cat > "$test_root/bin/systemd-sysusers" <<'EOF'
#!/bin/sh
exit 0
EOF
cat > "$test_root/bin/systemd-tmpfiles" <<'EOF'
#!/bin/sh
# The real permission and symlink behavior is checked in the install container.
printf '%s\n' "$*" >> "$HOOK_TEST_PERMISSION_LOG"
exit 0
EOF
cat > "$test_root/bin/id" <<'EOF'
#!/bin/sh
case "$1" in -u|-g) echo "${HOOK_TEST_BAD_ACCOUNT:-999}" ;; -gn) echo fleetint-oob ;; *) exit 1 ;; esac
EOF
chmod +x "$test_root/bin/systemd-sysusers" "$test_root/bin/systemd-tmpfiles" "$test_root/bin/id"

run_hook() {
    : > "$HOOK_TEST_LOG"
    : > "$HOOK_TEST_PERMISSION_LOG"
    # Redirect only filesystem paths; execute the actual maintainer hook body.
    sed -e "s|/run/systemd/system|$test_root/systemd|g" \
        -e "s|/etc/fleetint-oob/service.conf|$test_root/etc/fleetint-oob/service.conf|g" \
        "$hook_script_root/$1" > "$test_root/hook"
    sh "$test_root/hook" "$2"
}

units='fleetint-oob.target fleetint-oob-collector.service fleetint-nico-health.service fleetint-otel-gateway.service'

run_hook deb/postinst configure
grep -Fx -- '--create /usr/lib/fleetint-oob/permissions.conf' "$HOOK_TEST_PERMISSION_LOG"
if grep -Eq ' enable | start |restart' "$HOOK_TEST_LOG"; then
    echo 'Fresh installation enabled or started a service' >&2; exit 1
fi
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
if grep -q 'disable' "$HOOK_TEST_LOG"; then
    echo 'Debian removal discarded enablement state' >&2; exit 1
fi
run_hook deb/prerm failed-upgrade
grep -q 'deb-systemd-invoke stop' "$HOOK_TEST_LOG"

run_hook deb/postrm remove
grep -Fx "deb-systemd-helper mask $units" "$HOOK_TEST_LOG"
run_hook deb/postrm purge
grep -Fx "deb-systemd-helper purge $units" "$HOOK_TEST_LOG"
grep -Fx "deb-systemd-helper unmask $units" "$HOOK_TEST_LOG"

export DPKG_ROOT="$test_root/alternate-root"
mkdir -p "$DPKG_ROOT/etc"
printf '%s\n' 'fleetint-oob:x:1234:2345:OOB:/nonexistent:/usr/sbin/nologin' > "$DPKG_ROOT/etc/passwd"
printf '%s\n' 'fleetint-oob:x:2345:' > "$DPKG_ROOT/etc/group"
test ! -e "$DPKG_ROOT/usr/bin/id"
run_hook deb/postinst configure
grep -Fx -- "--root=$DPKG_ROOT --create /usr/lib/fleetint-oob/permissions.conf" "$HOOK_TEST_PERMISSION_LOG"
if grep -q '^systemctl' "$HOOK_TEST_LOG"; then
    echo 'Alternate-root installation contacted host systemd' >&2; exit 1
fi
run_hook deb/prerm remove
test ! -s "$HOOK_TEST_LOG"
run_hook deb/postrm remove
if grep -q '^systemctl' "$HOOK_TEST_LOG"; then
    echo 'A hook contacted systemd without a running instance' >&2; exit 1
fi
unset DPKG_ROOT

run_hook rpm/postinstall 1
grep -Fx -- '--create /usr/lib/fleetint-oob/permissions.conf' "$HOOK_TEST_PERMISSION_LOG"
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
if grep -q '^systemctl' "$HOOK_TEST_LOG"; then
    echo 'A hook contacted systemd without a running instance' >&2; exit 1
fi
run_hook rpm/preremove 0
grep -q 'disable' "$HOOK_TEST_LOG"
if grep -q 'systemctl stop' "$HOOK_TEST_LOG"; then
    echo 'A hook stopped services without a running systemd instance' >&2; exit 1
fi
# Deleted conffiles remain deleted on upgrade, without breaking dpkg/rpm.
rm "$test_root/etc/fleetint-oob/service.conf"
run_hook deb/postinst configure
run_hook rpm/postinstall 2
test ! -e "$test_root/etc/fleetint-oob/service.conf"
echo 'Package lifecycle hook checks passed.'

export HOOK_TEST_BAD_ACCOUNT=0
if run_hook deb/postinst configure; then
    echo 'Accepted a root service account' >&2; exit 1
fi
if run_hook rpm/postinstall 1; then
    echo 'Accepted a root service account in an RPM install' >&2; exit 1
fi
unset HOOK_TEST_BAD_ACCOUNT
export DPKG_ROOT="$test_root/alternate-root"
for uid in 0 0000 invalid ''; do
    printf 'fleetint-oob:x:%s:2345:OOB:/nonexistent:/usr/sbin/nologin\n' "$uid" > "$DPKG_ROOT/etc/passwd"
    if run_hook deb/postinst configure; then
        echo 'Accepted an invalid service UID in an alternate root' >&2; exit 1
    fi
done
printf '%s\n' 'fleetint-oob:x:1234:2345:OOB:/nonexistent:/usr/sbin/nologin' > "$DPKG_ROOT/etc/passwd"
for group in 'fleetint-oob:x:0:' 'fleetint-oob:x:3456:' 'other-group:x:2345:' ''; do
    printf '%s\n' "$group" > "$DPKG_ROOT/etc/group"
    if run_hook deb/postinst configure; then
        echo 'Accepted a missing or mismatched primary group in an alternate root' >&2; exit 1
    fi
done
printf '%s\n' 'fleetint-oob:x:2345:' > "$DPKG_ROOT/etc/group"
printf '%s\n' 'other-user:x:1234:2345:OOB:/nonexistent:/usr/sbin/nologin' > "$DPKG_ROOT/etc/passwd"
if run_hook deb/postinst configure; then
    echo 'Accepted a missing service account in an alternate root' >&2; exit 1
fi
unset DPKG_ROOT
