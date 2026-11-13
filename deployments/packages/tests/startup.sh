#!/bin/sh
# Copyright (c) 2026, NVIDIA CORPORATION & AFFILIATES. All rights reserved.
# SPDX-License-Identifier: Apache-2.0
set -eu

launcher=${1:-deployments/packages/run-service}
unit_root=${2:-deployments/packages/systemd}
test_root=$(mktemp -d)
trap 'rm -rf "$test_root"' EXIT HUP INT TERM
: > "$test_root/empty-token"

expect_missing_token() {
    status=0
    env -u FLEETINT_ENROLLMENT_TOKEN -u FLEETINT_ENROLLMENT_TOKEN_FILE "$@" \
        "$shell" "$launcher" "$service" > "$test_root/stdout" 2> "$test_root/stderr" || status=$?
    if [ "$status" -ne 2 ]; then
        echo "Missing token returned $status under $shell for $service; expected 2" >&2
        exit 1
    fi
    test -s "$test_root/stderr"
}

for shell in sh bash; do
    for service in collector gateway; do
        expect_missing_token
        expect_missing_token FLEETINT_ENROLLMENT_TOKEN=
        expect_missing_token FLEETINT_ENROLLMENT_TOKEN_FILE="$test_root/empty-token" \
            FLEETINT_ENROLLMENT_TOKEN=environment-token
        expect_missing_token FLEETINT_ENROLLMENT_TOKEN_FILE="$test_root/missing-token" \
            FLEETINT_ENROLLMENT_TOKEN=environment-token
    done
done
for unit in fleetint-oob-collector.service fleetint-otel-gateway.service fleetint-nico-health.service; do
    if ! grep -Fxq 'RestartPreventExitStatus=2' "$unit_root/$unit"; then
        echo "$unit must prevent retries for configuration failures" >&2
        exit 1
    fi
    grep -Fxq 'Restart=on-failure' "$unit_root/$unit"
done
printf 'Missing-token exit status and service restart checks passed.\n'
