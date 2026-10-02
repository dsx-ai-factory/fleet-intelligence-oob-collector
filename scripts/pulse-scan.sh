#!/usr/bin/env bash
set -euo pipefail

for required in NGC_API_KEY SSA_CLIENT_ID SSA_CLIENT_SECRET NSPECT_ID GHCR_USERNAME GHCR_TOKEN; do
  if [[ -z "${!required:-}" ]]; then
    echo "Missing required setting: $required" >&2
    exit 1
  fi
done

image="${1:?provide an image reference}"
platform="${2:?provide a platform}"
reports="${3:?provide a report directory}"
scanner="${PULSE_SCANNER_IMAGE:-nvcr.io/0898940053369630/pulse/pulse-container-scanner-cli:stable}"
repo_root="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
docker_config="$(mktemp -d)"
trap 'rm -rf "$docker_config"' EXIT
export DOCKER_CONFIG="$docker_config"

printf '%s' "$NGC_API_KEY" | docker login nvcr.io --username '$oauthtoken' --password-stdin
printf '%s' "$GHCR_TOKEN" | docker login ghcr.io --username "$GHCR_USERNAME" --password-stdin

token_response="$(curl --fail --silent --show-error --connect-timeout 10 --max-time 60 \
  --request POST \
  --user "$SSA_CLIENT_ID:$SSA_CLIENT_SECRET" \
  --data-urlencode 'grant_type=client_credentials' \
  --data-urlencode 'scope=nspect.verify scan.anchore' \
  'https://x9thwm-cootr2q1jdv5p7b8iw4fs4ob3x6nqqsoznyk.ssa.nvidia.com/token')"
SSA_TOKEN="$(printf '%s' "$token_response" | python3 -c '
import json, sys
try:
    token = json.load(sys.stdin)["access_token"]
    if not isinstance(token, str) or not token or any(c.isspace() for c in token):
        raise ValueError()
except (ValueError, KeyError, TypeError):
    sys.exit("SSA response did not contain a valid access token")
print(token)
')"
export SSA_TOKEN
if [[ "${GITHUB_ACTIONS:-}" == true ]]; then
  echo "::add-mask::$SSA_TOKEN"
fi

mkdir -p "$reports"
reports="$(cd "$reports" && pwd)"
docker run --rm \
  --user 0:0 \
  -e SSA_TOKEN \
  -e REGISTRY_AUTH_FILE=/root/.docker/config.json \
  -v "$docker_config:/root/.docker:ro" \
  -v "$repo_root/security/pulse/policy.json:/policy.json:ro" \
  -v "$reports:/reports" \
  -w /reports \
  --entrypoint /bin/sh \
  "$scanner" -c '
    if [ -r /home/scanner/.anchorectl.yaml ]; then
      export ANCHORECTL_CONFIG=/home/scanner/.anchorectl.yaml
    fi
    exec pulse-cli "$@"
  ' pulse-scan -n "$NSPECT_ID" scan-image \
    -i "$image" --platform "$platform" -p /policy.json -o
