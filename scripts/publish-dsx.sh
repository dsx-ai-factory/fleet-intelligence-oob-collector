#!/usr/bin/env bash
# Copyright (c) 2026, NVIDIA CORPORATION & AFFILIATES. All rights reserved.
# SPDX-License-Identifier: Apache-2.0

set -euo pipefail

: "${VERSION:?Set the released version without its leading v}"
: "${DSX_NGC_ORG_TEAM:?Set the destination org/team}"
: "${DSX_KEY:?Configure DSX_NGC_API_KEY in the publishing environment}"
: "${IMAGE:?Set the released collector repository}"
: "${NICO_IMAGE:?Set the released NICo repository}"
[[ "$VERSION" =~ ^[0-9]+\.[0-9]+\.[0-9]+(-[0-9A-Za-z]+([.][0-9A-Za-z]+)*)?$ ]] || { echo 'Invalid release version' >&2; exit 1; }
[[ "$DSX_NGC_ORG_TEAM" =~ ^[a-z0-9][a-z0-9_-]*/[a-z0-9][a-z0-9_-]*$ ]] || { echo 'DSX_NGC_ORG_TEAM must be an NGC org/team path' >&2; exit 1; }
prefix="${DSX_ARTIFACT_PREFIX:-}"
[[ -z "$prefix" || "$prefix" =~ ^dsx-test-[0-9]+-[0-9]+-$ ]] || { echo 'Invalid isolated test prefix' >&2; exit 1; }
chart=$(realpath "${1:?Pass the packaged chart path}")
chart_name="${prefix}fleet-intelligence-oob-collector"
[[ "${chart##*/}" == "$chart_name-$VERSION.tgz" ]] || { echo 'Unexpected chart archive name' >&2; exit 1; }

auth_dir=$(mktemp -d "${RUNNER_TEMP:-${TMPDIR:-/tmp}}/dsx-copy.XXXXXX")
trap 'rm -rf "$auth_dir"' EXIT
# Separate credentials are required even though both registries use nvcr.io.
printf '{"auths":{}}\n' > "$auth_dir/source.json"
if [[ -n "${SOURCE_KEY:-}" ]]; then
  printf '%s' "$SOURCE_KEY" | skopeo login --authfile "$auth_dir/source.json" --username '$oauthtoken' --password-stdin nvcr.io
fi
printf '%s' "$DSX_KEY" | skopeo login --authfile "$auth_dir/destination.json" --username '$oauthtoken' --password-stdin nvcr.io

gateway_image=$(helm template dsx-publish "$chart" \
  --set-json 'inventory.targets=[{"address":"192.0.2.10","mac":"02:00:00:00:00:01"}]' \
  --show-only templates/otel-gateway-deployment.yaml | sed -n 's/^[[:space:]]*image: "\([^"]*\)"$/\1/p')
[[ "$gateway_image" == nvcr.io/nvidia/fleet-intelligence/otel-gateway:* && "$gateway_image" != *$'\n'* ]] || { echo 'Expected one FI gateway image in the packaged chart' >&2; exit 1; }

for source in "$IMAGE:$VERSION" "$NICO_IMAGE:$VERSION" "$gateway_image"; do
  repository="${source%:*}"
  destination="nvcr.io/$DSX_NGC_ORG_TEAM/${prefix}fleet-intelligence-${repository##*/}:${source##*:}"
  digest=$(skopeo inspect --raw --authfile "$auth_dir/source.json" "docker://$source" | sha256sum | cut -d ' ' -f 1)
  if skopeo inspect --raw --authfile "$auth_dir/destination.json" "docker://$destination" > "$auth_dir/manifest" 2> "$auth_dir/error"; then
    [[ "$(sha256sum "$auth_dir/manifest" | cut -d ' ' -f 1)" == "$digest" ]] || { echo "::error::$destination already contains different content"; exit 1; }
  else
    if grep -Eqi 'unauthorized|denied|forbidden|authentication|timeout|timed out|deadline|certificate|tls|connection|network|unavailable|too many requests' "$auth_dir/error" || ! grep -Eqi '(^|: )(manifest unknown|name unknown)(:|$)' "$auth_dir/error"; then
      echo '::error::Cannot inspect destination image'
      sed 's/^/Registry inspection: /' "$auth_dir/error" >&2
      exit 1
    fi
    skopeo copy --all --preserve-digests \
      --src-authfile "$auth_dir/source.json" --dest-authfile "$auth_dir/destination.json" \
      "docker://$repository@sha256:$digest" "docker://$destination"
  fi
  actual=$(skopeo inspect --raw --authfile "$auth_dir/destination.json" "docker://$destination" | sha256sum | cut -d ' ' -f 1)
  [[ "$actual" == "$digest" ]] || { echo "::error::Digest verification failed for $destination"; exit 1; }
  printf '%s@sha256:%s\n' "$destination" "$actual"
done

export NGC_CLI_API_KEY="$DSX_KEY"
org="${DSX_NGC_ORG_TEAM%/*}"
team="${DSX_NGC_ORG_TEAM#*/}"
target="$DSX_NGC_ORG_TEAM/$chart_name"
cd "${chart%/*}"
if ! ngc registry chart info --org "$org" --team "$team" "$target"; then
  ngc registry chart create --org "$org" --team "$team" "$target" --short-desc "Fleet Intelligence out-of-band collector"
fi
# NGC returns failure for immutable duplicate versions and lost success responses.
# In every case, require a fresh download with exactly the expected archive bytes.
if ! ngc registry chart push --org "$org" --team "$team" "$target:$VERSION"; then
  echo 'Checking whether the destination already contains the expected chart'
fi
mkdir "$auth_dir/chart"
ngc registry chart pull --org "$org" --team "$team" --dest "$auth_dir/chart" "$target:$VERSION"
cmp "$chart" "$auth_dir/chart/${chart##*/}"
sha256sum "$auth_dir/chart/${chart##*/}"
