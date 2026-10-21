#!/bin/sh
# Copyright (c) 2026, NVIDIA CORPORATION & AFFILIATES. All rights reserved.
# SPDX-License-Identifier: Apache-2.0
set -eu

test_root=$(mktemp -d)
trap 'rm -rf "$test_root"' EXIT HUP INT TERM
mkdir -p "$test_root/bin" "$test_root/nico" "$test_root/agent/otelcol"
expect_failure() {
    if "$@" > "$test_root/failure.log" 2>&1; then
        echo "Unexpected success: $*" >&2; exit 1
    fi
}

# Test the assertion harness itself by introducing the forbidden behaviors.
cp -R deployments/packages/scripts "$test_root/hooks"
for action in enable start; do
    cp deployments/packages/scripts/deb/postinst "$test_root/hooks/deb/postinst"
    printf '\nsystemctl %s fleetint-oob.target\n' "$action" >> "$test_root/hooks/deb/postinst"
    expect_failure env HOOK_SCRIPT_ROOT="$test_root/hooks" make --no-print-directory packages-test-hooks
    grep -q 'Fresh installation enabled or started' "$test_root/failure.log"
done

expect_failure make --no-print-directory packages-check-deb OUTPUTDIR="$test_root/empty"
grep -q 'No DEB packages' "$test_root/failure.log"
expect_failure make --no-print-directory packages-check-rpm OUTPUTDIR="$test_root/empty"
grep -q 'No RPM packages' "$test_root/failure.log"

# Real local Git repositories, with mocked compilers: no network or actual builds.
printf '[workspace]\n' > "$test_root/nico/Cargo.toml"
printf 'dist: {}\n' > "$test_root/agent/otelcol/otelcol-builder.yaml"
for project in nico agent; do
    printf 'test license\n' > "$test_root/$project/LICENSE"
    printf 'test notices\n' > "$test_root/$project/NOTICE"
    printf 'test notices\n' > "$test_root/$project/THIRD-PARTY-LICENSES"
    git -C "$test_root/$project" init -q
    git -C "$test_root/$project" add .
    git -C "$test_root/$project" -c user.name='Package Tests' -c user.email=tests@example.com commit -qm first
    git -C "$test_root/$project" rev-parse HEAD > "$test_root/$project-first"
    printf 'updated\n' >> "$test_root/$project/LICENSE"
    git -C "$test_root/$project" add .
    git -C "$test_root/$project" -c user.name='Package Tests' -c user.email=tests@example.com commit -qm second
    git -C "$test_root/$project" rev-parse HEAD > "$test_root/$project-second"
done
export GIT_CONFIG_COUNT=2
export GIT_CONFIG_KEY_0="url.file://$test_root/nico.insteadOf"
export GIT_CONFIG_VALUE_0=https://github.com/NVIDIA/infra-controller.git
export GIT_CONFIG_KEY_1="url.file://$test_root/agent.insteadOf"
export GIT_CONFIG_VALUE_1=https://github.com/dsx-ai-factory/fleet-intelligence-agent.git

cat > "$test_root/bin/cargo" <<'EOF'
#!/bin/sh
set -eu
while [ "$1" != --target ]; do shift; done
target=$2
mkdir -p "$CARGO_TARGET_DIR/$target/release"
git rev-parse HEAD > "$CARGO_TARGET_DIR/$target/release/forge-hw-health"
EOF
cat > "$test_root/bin/go" <<'EOF'
#!/bin/sh
set -eu
for arg do
    case "$arg" in --output-path=*) output=${arg#--output-path=} ;; esac
done
mkdir -p "$output"
git rev-parse HEAD > "$output/fleetint-otelcol"
EOF
printf '#!/bin/sh\nexit 0\n' > "$test_root/bin/rustup"
cp "$test_root/bin/rustup" "$test_root/bin/protoc"
chmod +x "$test_root/bin/"*
export PATH="$test_root/bin:$PATH"

for revision in first second; do
    nico_ref=$(cat "$test_root/nico-$revision")
    agent_ref=$(cat "$test_root/agent-$revision")
    make --no-print-directory package-nico package-gateway \
        NICO_REF="$nico_ref" AGENT_REF="$agent_ref" \
        PACKAGE_SOURCEDIR="$test_root/sources" PACKAGE_BUILDDIR="$test_root/build" \
        INPUTDIR="$test_root/input" > "$test_root/build.log" 2>&1
    test "$(cat "$test_root/input/nico-hardware-health")" = "$nico_ref"
    test "$(cat "$test_root/input/otel-gateway")" = "$agent_ref"
done
expect_failure make --no-print-directory package-nico \
    NICO_REF="$nico_ref" NICO_SOURCE="$test_root/sources/infra-controller-$(cat "$test_root/nico-first")"
grep -q 'NICo checkout does not match' "$test_root/failure.log"
expect_failure make --no-print-directory package-gateway \
    AGENT_REF="$agent_ref" AGENT_SOURCE="$test_root/sources/fleet-intelligence-agent-$(cat "$test_root/agent-first")"
grep -q 'Agent checkout does not match' "$test_root/failure.log"

# Native arm64 defaults must not silently request amd64.
cat > "$test_root/bin/uname" <<'EOF'
#!/bin/sh
case "$1" in -m) echo aarch64 ;; -s) echo Linux ;; *) exit 1 ;; esac
EOF
chmod +x "$test_root/bin/uname"
make --no-print-directory package-native-check
make --no-print-directory -n package-collector > "$test_root/arm64.log"
grep -q 'aarch64-unknown-linux-musl' "$test_root/arm64.log"

# Never ship an ELF interpreter or a shared-library dependency, including OpenSSL.
cat > "$test_root/bin/readelf" <<'EOF'
#!/bin/sh
if [ "${TEST_DYNAMIC:-0}" = 1 ]; then
    echo ' 0x0000000000000001 (NEEDED) Shared library: [libssl.so.3]'
fi
EOF
chmod +x "$test_root/bin/readelf"
make --no-print-directory package-static-check INPUTDIR="$test_root/input"
expect_failure env TEST_DYNAMIC=1 make --no-print-directory package-static-check INPUTDIR="$test_root/input"
grep -q 'dynamically linked; refusing' "$test_root/failure.log"
echo 'Package build regression checks passed.'
