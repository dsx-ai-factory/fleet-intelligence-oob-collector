#!/bin/sh
# Copyright (c) 2026, NVIDIA CORPORATION & AFFILIATES. All rights reserved.
# SPDX-License-Identifier: Apache-2.0
set -eu

launcher=${1:-deployments/packages/run-service}
unit_root=${2:-deployments/packages/systemd}
test_root=$(mktemp -d)
trap 'rm -rf "$test_root"' EXIT HUP INT TERM
: > "$test_root/empty-service-key"

expect_missing_service_key() {
    status=0
    env -u FLEETINT_SERVICE_KEY -u FLEETINT_SERVICE_KEY_FILE "$@" \
        "$shell" "$launcher" "$service" > "$test_root/stdout" 2> "$test_root/stderr" || status=$?
    if [ "$status" -ne 2 ]; then
        echo "Missing service key returned $status under $shell for $service; expected 2" >&2
        exit 1
    fi
    test -s "$test_root/stderr"
}

for shell in sh bash; do
    for service in collector gateway; do
        expect_missing_service_key
        expect_missing_service_key FLEETINT_SERVICE_KEY=
        expect_missing_service_key FLEETINT_SERVICE_KEY_FILE="$test_root/empty-service-key" \
            FLEETINT_SERVICE_KEY=environment-token
        expect_missing_service_key FLEETINT_SERVICE_KEY_FILE="$test_root/missing-service-key" \
            FLEETINT_SERVICE_KEY=environment-token
    done
done
for unit in fleetint-oob-collector.service fleetint-otel-gateway.service fleetint-nico-health.service; do
    if ! grep -Fxq 'RestartPreventExitStatus=2' "$unit_root/$unit"; then
        echo "$unit must prevent retries for configuration failures" >&2
        exit 1
    fi
    grep -Fxq 'Restart=on-failure' "$unit_root/$unit"
done
printf 'Missing service key exit status and service restart checks passed.\n'
