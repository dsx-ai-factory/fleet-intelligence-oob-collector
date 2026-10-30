#!/usr/bin/env bash
# Copyright (c) 2026, NVIDIA CORPORATION & AFFILIATES. All rights reserved.
# SPDX-License-Identifier: Apache-2.0
set -euo pipefail
IFS= read -r request
request=${request%$'\r'}
authorization=
length=0
while IFS= read -r header; do
    header=${header%$'\r'}
    [[ -n "$header" ]] || break
    case "${header,,}" in
        'authorization: '*) authorization=${header#*: } ;;
        'content-length: '*) length=${header#*: } ;;
    esac
done
[[ "$length" =~ ^[0-9]+$ ]] && (( length <= 65536 ))
if (( length > 0 )); then dd bs=1 count="$length" status=none > /dev/null; fi
reply() {
    local body=$1
    printf 'HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: %s\r\nConnection: close\r\n\r\n%s' "${#body}" "$body"
}
if [[ "$request" == 'POST /v1/agent/enroll HTTP/1.1' && "$authorization" == 'Bearer test-token' ]]; then
    reply '{"jwtAssertion":"test-runtime-jwt"}'
    touch "$PACKAGE_TEST_DIRECTORY/collector-enrolled"
elif [[ "$request" == 'POST /api/v1/health/enroll HTTP/1.1' && "$authorization" == 'Bearer test-token' ]]; then
    reply '{"jwtAssertion":"test-runtime-jwt"}'
elif [[ "$request" == 'PUT /v1/agent/oob_nodes HTTP/1.1' && "$authorization" == 'Bearer test-runtime-jwt' ]]; then
    reply '{"results":[{"status":"OK"}]}'
    touch "$PACKAGE_TEST_DIRECTORY/collector-uploaded"
elif [[ "$request" == 'POST /api/v1/health/metrics HTTP/1.1' && "$authorization" == 'Bearer test-runtime-jwt' ]]; then
    reply '{}'
    touch "$PACKAGE_TEST_DIRECTORY/gateway-exported"
else
    printf 'HTTP/1.1 500 Internal Server Error\r\nContent-Length: 0\r\nConnection: close\r\n\r\n'
fi
