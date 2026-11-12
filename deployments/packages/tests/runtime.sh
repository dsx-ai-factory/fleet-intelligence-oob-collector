#!/usr/bin/env bash
# Copyright (c) 2026, NVIDIA CORPORATION & AFFILIATES. All rights reserved.
# SPDX-License-Identifier: Apache-2.0
set -euo pipefail
work=$(mktemp -d /tmp/package-runtime.XXXXXXXX)
export PACKAGE_TEST_DIRECTORY="$work"
nico_pid=
collector_pid=
backend_pid=
gateway_pid=
tls_pid=
credentials=
cleanup() {
    status=$?
    trap - EXIT
    if (( status != 0 )); then
        cat "$work/nico.log" "$work/collector.log" "$work/gateway.log" "$work/backend.log" 2>/dev/null || true
    fi
    for pid in "$nico_pid" "$collector_pid" "$gateway_pid" "$backend_pid" "$tls_pid"; do
        if [[ -n "$pid" ]] && kill -0 "$pid" 2>/dev/null; then
            kill -KILL "$pid" 2>/dev/null || true
            wait "$pid" 2>/dev/null || true
        fi
    done
    [[ -z "$credentials" ]] || rm -rf "$credentials"
    rm -rf "$work"
    exit "$status"
}
trap cleanup EXIT
# Notices are consolidated files, with no duplicate dependency license trees.
[[ ! -e /usr/share/doc/fleetint-oob/collector-dependencies ]]
[[ ! -e /usr/share/doc/fleetint-oob/nico-dependencies ]]
grep -Fq 'NICo hardware health (carbide-health)' /usr/share/doc/fleetint-oob/NOTICE
wait_for() {
    local pid=$1
    shift
    for ((attempt=0; attempt<60; attempt++)); do
        kill -0 "$pid"
        if "$@"; then return 0; fi
        sleep 1
    done
    echo 'Daemon did not become ready' >&2
    return 1
}
stop_daemon() {
    local pid=$1 signal=$2
    kill -"$signal" "$pid" || return
    for ((attempt=0; attempt<30; attempt++)); do
        if ! kill -0 "$pid" 2>/dev/null; then
            wait "$pid"
            return
        fi
        sleep 1
    done
    echo 'Daemon did not stop within 30 seconds' >&2
    return 1
}
setpriv --reuid=fleetint-oob --regid=fleetint-oob --clear-groups -- \
    env FLEETINT_OTLP_ENDPOINT=http://127.0.0.1:4317 \
    /usr/lib/fleetint-oob/run-nico > "$work/nico.log" 2>&1 &
nico_pid=$!
wait_for "$nico_pid" curl --fail --silent --max-time 1 http://127.0.0.1:9009/livez
curl --fail --silent --max-time 5 http://127.0.0.1:9009/metrics > /dev/null
directory=/run/fleetint-nico-health
[[ $(stat -c %a "$directory") == 700 ]]
configs=("$directory"/config.*/config.toml)
[[ ${#configs[@]} == 1 ]]
# The real NICo binary must accept the overrides produced by the shared renderer.
grep -Fxq 'bmc_request_concurrency = 2' "${configs[0]}"
grep -Fxq '[collectors.sensors]' "${configs[0]}"
grep -Fxq 'sensor_fetch_interval = "120s"' "${configs[0]}"
grep -Fxq '[collectors.leak_detector]' "${configs[0]}"
grep -Fxq 'poll_interval = "180s"' "${configs[0]}"
grep -Fxq '[collectors.logs.sse]' "${configs[0]}"
grep -Fxq 'max_backoff = "60s"' "${configs[0]}"
[[ $(stat -c %a "${configs[0]}") == 640 ]]
[[ $(stat -c %u "${configs[0]}") == "$(id -u fleetint-oob)" ]]
# NICo may handle TERM or use the default signal disposition.
if stop_daemon "$nico_pid" TERM; then :; else [[ $? == 143 ]]; fi
nico_pid=
if grep -Fq test-password "$work/nico.log"; then
    echo 'NICo logged a BMC password' >&2; exit 1
else
    [[ $? == 1 ]]
fi
# The fixture and collector only communicate over container loopback.
socat -T 5 TCP4-LISTEN:18888,bind=127.0.0.1,reuseaddr,fork EXEC:/backend.sh > "$work/backend.log" 2>&1 &
backend_pid=$!
# Trust an ephemeral loopback certificate. The real gateway still verifies TLS;
# the fixture server uses ordinary HTTPS without mutual TLS client certificates.
if command -v update-ca-certificates >/dev/null 2>&1; then
    certificate=/usr/local/share/ca-certificates/fleetint-test.crt
else
    certificate=/etc/pki/ca-trust/source/anchors/fleetint-test.crt
fi
openssl req -x509 -newkey rsa:2048 -nodes -days 1 -subj /CN=fleetint-package-test \
    -addext subjectAltName=IP:127.0.0.1 -addext basicConstraints=critical,CA:TRUE \
    -keyout "$work/backend-test.key" -out "$certificate" > "$work/tls.log" 2>&1
if command -v update-ca-certificates >/dev/null 2>&1; then
    update-ca-certificates >> "$work/tls.log" 2>&1
else
    update-ca-trust >> "$work/tls.log" 2>&1
fi
socat -T 5 "OPENSSL-LISTEN:18889,bind=127.0.0.1,reuseaddr,fork,cert=$certificate,key=$work/backend-test.key,verify=0" EXEC:/backend.sh >> "$work/backend.log" 2>&1 &
tls_pid=$!
wait_for "$tls_pid" bash -c 'exec 3<>/dev/tcp/127.0.0.1/18889' 2>/dev/null
wait_for "$backend_pid" bash -c 'exec 3<>/dev/tcp/127.0.0.1/18888' 2>/dev/null
credentials=$(mktemp -d /tmp/collector-credentials.XXXXXX)
printf 'test-token\n' > "$credentials/agent-token"
chmod 640 "$credentials/agent-token"
chown root:fleetint-oob "$credentials" "$credentials/agent-token"
chmod 750 "$credentials"
start=$(sed -n 's/^ExecStart=//p' /usr/lib/systemd/system/fleetint-oob-collector.service)
start=${start//\$\$/\$}
# Parse the installed unit's shell quoting; exec preserves the collector PID.
setpriv --reuid=fleetint-oob --regid=fleetint-oob --clear-groups -- \
    env FLEETINT_AGENT_TOKEN_FILE="$credentials/agent-token" FLEETINT_AGENT_TOKEN=wrong-legacy-token FLEETINT_DISABLE_LATENCY_PROBE=true \
    FLEETINT_BACKEND_URL=http://127.0.0.1:18888 \
    /bin/sh -ec "exec $start" > "$work/collector.log" 2>&1 &
collector_pid=$!
wait_for "$collector_pid" test -e "$work/collector-enrolled"
tr '\0' ' ' < "/proc/$collector_pid/cmdline" > "$work/collector-arguments"
if grep -Fq test-token "$work/collector-arguments"; then
    echo 'Collector exposed its token in process arguments' >&2; exit 1
else
    [[ $? == 1 ]]
fi
# This fixture deliberately uses an unavailable BMC. The collector now holds
# failed targets for its retry delay; stopping it must flush that inventory.
wait_for "$collector_pid" grep -Fq 'retrying failed inventory targets after a delay' "$work/collector.log"
# Gateway configuration is validated and its real health endpoint is probed.
setpriv --reuid=fleetint-oob --regid=fleetint-oob --clear-groups -- \
    env FLEETINT_AGENT_TOKEN=test-token BACKEND_ENDPOINT=https://127.0.0.1:18889/api/v1/health \
    ENROLL_ENDPOINT=https://127.0.0.1:18889/api/v1/health/enroll \
    /usr/lib/fleetint-oob/run-service gateway > "$work/gateway.log" 2>&1 &
gateway_pid=$!
wait_for "$gateway_pid" curl --fail --silent --max-time 1 http://127.0.0.1:13133/
curl --fail --silent --max-time 5 -H 'Content-Type: application/json' \
    --data '{"resourceMetrics":[{"scopeMetrics":[{"metrics":[{"name":"packaging_test","gauge":{"dataPoints":[{"timeUnixNano":"1700000000000000000","asDouble":1}]}}]}]}]}' \
    http://127.0.0.1:4318/v1/metrics > /dev/null
wait_for "$gateway_pid" test -e "$work/gateway-exported"
stop_daemon "$gateway_pid" TERM
gateway_pid=
stop_daemon "$collector_pid" INT
collector_pid=
test -e "$work/collector-uploaded"
printf 'Installed daemon runtime checks passed\n'
