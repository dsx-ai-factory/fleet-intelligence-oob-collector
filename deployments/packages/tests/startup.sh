#!/bin/sh
# Copyright (c) 2026, NVIDIA CORPORATION & AFFILIATES. All rights reserved.
# SPDX-License-Identifier: Apache-2.0
set -eu

launcher=${1:-deployments/packages/run-service}
unit_root=${2:-deployments/packages/systemd}
test_root=$(mktemp -d)
trap 'rm -rf "$test_root"' EXIT HUP INT TERM
: > "$test_root/empty-token"
printf ' \t\r\n' > "$test_root/whitespace-token"

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
        expect_missing_token "FLEETINT_ENROLLMENT_TOKEN=$(printf ' \t\r')"
        expect_missing_token FLEETINT_ENROLLMENT_TOKEN_FILE="$test_root/whitespace-token" \
            FLEETINT_ENROLLMENT_TOKEN=fallback-token
        expect_missing_token FLEETINT_ENROLLMENT_TOKEN_FILE="$test_root/empty-token" \
            FLEETINT_ENROLLMENT_TOKEN=environment-token
        expect_missing_token FLEETINT_ENROLLMENT_TOKEN_FILE="$test_root/missing-token" \
            FLEETINT_ENROLLMENT_TOKEN=environment-token
    done
done
# Replace only executable paths; exercise the real launcher without starting daemons.
cat > "$test_root/daemon" <<'DAEMON'
#!/bin/sh
printf 'token=<%s>\n' "${FLEETINT_ENROLLMENT_TOKEN-}"
printf 'arg=<%s>\n' "$@"
DAEMON
chmod 755 "$test_root/daemon"
sed -e "s|/usr/bin/bmc-collector|$test_root/daemon|g" \
    -e "s|/usr/lib/fleetint-oob/otel-gateway|$test_root/daemon|g" \
    "$launcher" > "$test_root/launcher"
expect_normalized_token() {
    expected=$1
    shift
    env -u FLEETINT_ENROLLMENT_TOKEN -u FLEETINT_ENROLLMENT_TOKEN_FILE "$@" \
        "$shell" "$test_root/launcher" "$service" > "$test_root/stdout" 2> "$test_root/stderr"
    printf 'token=<%s>\n' "$expected" > "$test_root/expected"
    sed -n '1p' "$test_root/stdout" > "$test_root/actual"
    cmp "$test_root/expected" "$test_root/actual"
    sed '1d' "$test_root/stdout" > "$test_root/arguments"
    if grep -Fq -- "$expected" "$test_root/arguments"; then
        echo 'Token was exposed in daemon arguments' >&2; exit 1
    fi
    test ! -s "$test_root/stderr"
}
for shell in sh bash; do
    for service in collector gateway; do
        for padding in none lf crlf surrounding repeated; do
            case "$padding" in
                none) printf 'example-token' ;;
                lf) printf 'example-token\n' ;;
                crlf) printf 'example-token\r\n' ;;
                surrounding) printf ' \t\r\nexample-token \t\r\n' ;;
                repeated) printf '\n\nexample-token\n\n' ;;
            esac > "$test_root/token"
            expect_normalized_token example-token \
                FLEETINT_ENROLLMENT_TOKEN_FILE="$test_root/token" FLEETINT_ENROLLMENT_TOKEN=fallback-token
            # A sentinel keeps command substitution from consuming trailing newlines.
            value=$(cat "$test_root/token"; printf x)
            expect_normalized_token example-token "FLEETINT_ENROLLMENT_TOKEN=${value%x}"
        done
        expect_normalized_token 'first second' 'FLEETINT_ENROLLMENT_TOKEN= first second '
    done
done
for unit in fleetint-oob-collector.service fleetint-otel-gateway.service fleetint-nico-health.service; do
    if ! grep -Fxq 'RestartPreventExitStatus=2' "$unit_root/$unit"; then
        echo "$unit must prevent retries for configuration failures" >&2
        exit 1
    fi
    grep -Fxq 'Restart=on-failure' "$unit_root/$unit"
done
printf 'Token normalization, missing-token exit status and service restart checks passed.\n'
