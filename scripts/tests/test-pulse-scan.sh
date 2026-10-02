#!/usr/bin/env bash
set -euo pipefail

repo_root="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
test_root="$(mktemp -d)"
trap 'rm -rf "$test_root"' EXIT
mkdir -p "$test_root/bin"

cat > "$test_root/bin/docker" <<'MOCK'
#!/usr/bin/env bash
set -euo pipefail
printf 'docker\n' >> "$TEST_CASE/calls"
printf '%s\n' "$DOCKER_CONFIG" > "$TEST_CASE/config"
if [[ "$1" == run ]]; then
  printf '%s\0' "$@" > "$TEST_CASE/scan-args"
  printf '%s' "${SSA_TOKEN:-}" > "$TEST_CASE/token"
  exit "$SCAN_EXIT"
fi
cat > /dev/null
MOCK
cat > "$test_root/bin/curl" <<'MOCK'
#!/usr/bin/env bash
set -euo pipefail
printf 'curl\n' >> "$TEST_CASE/calls"
printf '%s\0' "$@" > "$TEST_CASE/curl-args"
printf '%s' "$TOKEN_RESPONSE"
exit "$CURL_EXIT"
MOCK
chmod 755 "$test_root/bin/docker" "$test_root/bin/curl"

fail() {
  echo "FAIL: $*" >&2
  exit 1
}

case_number=0
run_scan() {
  case_number=$((case_number + 1))
  export TEST_CASE="$test_root/case-$case_number"
  mkdir -p "$TEST_CASE"
  export NGC_API_KEY=ngc-key SSA_CLIENT_ID=client SSA_CLIENT_SECRET=secret
  export NSPECT_ID=program GHCR_USERNAME=user GHCR_TOKEN=ghcr-token
  export GITHUB_ACTIONS=true SCAN_EXIT="${1:-0}" CURL_EXIT="${4:-0}"
  local default_response='{"access_token":"test-token"}'
  export TOKEN_RESPONSE="${2-$default_response}"
  unset SSA_TOKEN
  if [[ -n "${3:-}" ]]; then
    unset "$3"
  fi
  if PATH="$test_root/bin:$PATH" bash "$repo_root/scripts/pulse-scan.sh" \
    'ghcr.io/example/image@sha256:aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa' \
    linux/arm64 "$TEST_CASE/reports" > "$TEST_CASE/stdout" 2> "$TEST_CASE/stderr"; then
    scan_status=0
  else
    scan_status=$?
  fi
  if [[ -f "$TEST_CASE/config" ]]; then
    config="$(cat "$TEST_CASE/config")"
    [[ ! -e "$config" ]] || fail "temporary registry credentials were not removed"
  fi
}

run_scan
[[ "$scan_status" == 0 ]] || fail "successful scan failed"
[[ "$(cat "$TEST_CASE/token")" == test-token ]] || fail "SSA token was not forwarded"
scan_args=()
while IFS= read -r -d '' arg; do scan_args+=("$arg"); done < "$TEST_CASE/scan-args"
expected=(pulse-scan -n program scan-image -i
  'ghcr.io/example/image@sha256:aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa'
  --platform linux/arm64 -p /policy.json -o)
offset=$((${#scan_args[@]} - ${#expected[@]}))
for ((i=0; i<${#expected[@]}; i++)); do
  [[ "${scan_args[offset+i]}" == "${expected[i]}" ]] || fail "scan target, platform, policy, or output differs"
done
has_user=false
has_registry=false
for ((i=0; i<${#scan_args[@]}; i++)); do
  if [[ "${scan_args[i]}" == --user && "${scan_args[i+1]}" == 0:0 ]]; then has_user=true; fi
  if [[ "${scan_args[i]}" == REGISTRY_AUTH_FILE=/root/.docker/config.json ]]; then has_registry=true; fi
done
[[ "$has_user" == true && "$has_registry" == true ]] || fail "registry mount permissions are not configured"
has_timeout=false
while IFS= read -r -d '' arg; do
  [[ "$arg" != --location && "$arg" != -L ]] || fail "SSA request follows redirects"
  if [[ "$arg" == --max-time ]]; then has_timeout=true; fi
done < "$TEST_CASE/curl-args"
[[ "$has_timeout" == true ]] || fail "SSA request has no timeout"
echo 'PASS: exact digest/platform, policy, reports, and token forwarding'

for status in 1 255; do
  run_scan "$status"
  [[ "$scan_status" == "$status" ]] || fail "scanner exit $status was not propagated"
done
echo 'PASS: scanner failures propagate and credentials are cleaned up'

for response in '{}' 'not-json' '{"access_token":""}'; do
  run_scan 0 "$response"
  [[ "$scan_status" != 0 && ! -f "$TEST_CASE/scan-args" ]] || fail "invalid SSA response allowed a scan"
done
run_scan 0 '{"access_token":"test-token"}' '' 22
[[ "$scan_status" == 22 && ! -f "$TEST_CASE/scan-args" ]] || fail "failed SSA request allowed a scan"
echo 'PASS: invalid tokens and failed token requests prevent scans'

for credential in NGC_API_KEY SSA_CLIENT_ID SSA_CLIENT_SECRET NSPECT_ID GHCR_USERNAME GHCR_TOKEN; do
  run_scan 0 '{"access_token":"test-token"}' "$credential"
  [[ "$scan_status" != 0 && ! -f "$TEST_CASE/calls" ]] || fail "missing $credential allowed network access"
done
echo 'PASS: missing credentials prevent network access'

selector="$(sed -n 's/^    runs-on: //p' "$repo_root/.github/workflows/pulse-scan.yml")"
[[ "$selector" == *"vars.PULSE_RUNNER ||"* && "$selector" == *"fromJSON('[\"self-hosted\", \"rocky8\"]')"* ]] \
  || fail "scan runner does not default to the agent release pool"
echo 'PASS: agent release runner default and override'

for job in publish-image publish-nico-image; do
  if [[ "$job" == publish-image ]]; then build=build-image; else build=build-nico-image; fi
  expression="$(awk -v job="$job" '
    $0 == "  " job ":" { in_job=1; next }
    in_job && /runs-on:/ { exit }
    in_job && /if: >-/ { in_condition=1; next }
    in_condition { printf "%s ", $0 }
  ' "$repo_root/.github/workflows/release.yml")"
  expression="${expression#*\$\{\{}"
  expression="${expression%%\}\}*}"
  while read -r enabled scan_result build_result cancelled expected; do
    [[ "$enabled" != unset ]] || enabled=''
    evaluated="${expression//always()/1 == 1}"
    replacement="'$cancelled' == 'false'"
    evaluated="${evaluated//!cancelled()/$replacement}"
    replacement="'$build_result'"
    evaluated="${evaluated//needs.$build.result/$replacement}"
    replacement="'$scan_result'"
    evaluated="${evaluated//needs.pulse-scan.result/$replacement}"
    replacement="'$enabled'"
    evaluated="${evaluated//vars.PULSE_ENABLED/$replacement}"
    # Evaluate the condition from the actual workflow against release outcomes.
    if eval "[[ $evaluated ]]"; then actual=true; else actual=false; fi
    [[ "$actual" == "$expected" ]] || fail "$job gate: enabled=$enabled scan=$scan_result build=$build_result cancelled=$cancelled"
  done <<'CASES'
true success success false true
true failure success false false
true skipped success false false
true cancelled success false false
unset skipped success false true
false skipped success false true
unset skipped failure false false
true success failure false false
true success success true false
CASES
done
echo 'PASS: both release gates preserve disabled mode and block failed scans'
