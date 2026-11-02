#!/bin/sh
# Copyright (c) 2026, NVIDIA CORPORATION & AFFILIATES. All rights reserved.
# SPDX-License-Identifier: Apache-2.0
set -eu

test_root=$(mktemp -d)
trap 'rm -rf "$test_root"' EXIT HUP INT TERM
mkdir -p "$test_root/src" "$test_root/input"
cat > "$test_root/Cargo.toml" <<'MANIFEST'
[package]
name = "package-linking-test"
version = "0.0.0"
edition = "2024"
MANIFEST
cat > "$test_root/src/main.rs" <<'RUST'
fn main() {
    println!("Package toolchain executable ran successfully.");
}
RUST
CARGO_TARGET_DIR="$test_root/target" "$2" build --manifest-path "$test_root/Cargo.toml" \
    --target "$1" --release --offline
binary="$test_root/target/$1/release/package-linking-test"
for component in bmc-collector nico-hardware-health otel-gateway; do
    ln "$binary" "$test_root/input/$component"
done
make --no-print-directory package-static-check INPUTDIR="$test_root/input"
"$binary"
