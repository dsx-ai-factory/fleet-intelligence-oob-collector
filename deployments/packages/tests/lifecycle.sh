#!/bin/sh
# SPDX-License-Identifier: Apache-2.0
set -eu
work=$(mktemp -d /tmp/package-lifecycle.XXXXXX)
marker=/run/systemd/system
[ ! -e "$marker" ]
mkdir -p "$marker"
trap 'rm -rf "$work"; rmdir "$marker"' EXIT
cat > "$work/service-stub" <<'STUB'
#!/bin/sh
set -eu
name=${0##*/}
printf '%s %s\n' "$name" "$*" >> "$SERVICE_CALLS"
case "$name $1" in
    'systemctl is-active')
        [ "${FAIL_QUERY:-0}" != 1 ] || exit 2
        case " ${ACTIVE_SERVICES:-} " in *" $3 "*) exit 0;; *) exit 3;; esac ;;
    'deb-systemd-helper --quiet')
        case " ${ENABLED_SERVICES:-} " in *" $3 "*) exit 0;; *) exit 1;; esac ;;
    'deb-systemd-invoke restart') [ "${FAIL_RESTART:-0}" != 1 ] ;;
esac
STUB
chmod 755 "$work/service-stub"
for tool in systemctl deb-systemd-helper deb-systemd-invoke; do
    ln -s service-stub "$work/$tool"
done
export PATH="$work:$PATH" SERVICE_CALLS="$work/calls"
collector=fleetint-oob-collector.service
nico=fleetint-nico-health.service
gateway=fleetint-otel-gateway.service
run() {
    script=$1
    shift
    : > "$SERVICE_CALLS"
    timeout 15 /bin/sh "/var/lib/dpkg/info/fleetint-oob.$script" "$@" > "$work/stdout" 2> "$work/stderr"
    grep '^deb-systemd-invoke ' "$SERVICE_CALLS" > "$work/actions" || [ "$?" = 1 ]
}
run postinst configure ''
[ ! -s "$work/actions" ]
export ACTIVE_SERVICES="$collector" ENABLED_SERVICES="$collector"
run postinst configure 0.1.0
printf 'deb-systemd-invoke restart %s\n' "$collector" > "$work/expected"
cmp "$work/expected" "$work/actions"
grep -Fx "deb-systemd-helper enable $collector" "$SERVICE_CALLS"
if grep -Fx "deb-systemd-helper enable $nico" "$SERVICE_CALLS"; then
    echo 'Upgrade enabled a previously disabled service' >&2; exit 1
else
    [ "$?" = 1 ]
fi
export ACTIVE_SERVICES="$collector $nico $gateway" ENABLED_SERVICES="$collector $nico $gateway"
for _ in 1 2; do
    run postinst configure 0.1.0
    printf 'deb-systemd-invoke restart %s\n' "$collector" "$nico" "$gateway" > "$work/expected"
    cmp "$work/expected" "$work/actions"
done
export ACTIVE_SERVICES="$collector" FAIL_RESTART=1
run postinst configure 0.1.0
grep -F '[WARNING]' "$work/stderr" | grep -F "$collector"
unset FAIL_RESTART
export FAIL_QUERY=1
run postinst configure 0.1.0
[ ! -s "$work/actions" ]
grep -F '[WARNING]' "$work/stderr" | grep -F "$collector"
unset FAIL_QUERY
run prerm upgrade 0.2.0
[ ! -s "$work/actions" ]
run prerm remove
printf 'deb-systemd-invoke stop fleetint-oob.target %s %s %s\n' "$collector" "$nico" "$gateway" > "$work/expected"
cmp "$work/expected" "$work/actions"
run postrm purge
grep -Fx "deb-systemd-helper purge fleetint-oob.target $collector $nico $gateway" "$SERVICE_CALLS"
printf 'Service upgrade, failure reporting and removal policy checks passed\n'
