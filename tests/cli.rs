// Copyright (c) 2026, NVIDIA CORPORATION & AFFILIATES. All rights reserved.
// SPDX-License-Identifier: Apache-2.0

#[test]
fn runtime_help_shows_token_environment_name_without_its_value() {
    let output = std::process::Command::new(env!("CARGO_BIN_EXE_bmc-collector"))
        .args(["run", "--help"])
        .env("FLEETINT_AGENT_TOKEN", "example-private-token")
        .output()
        .expect("CLI help");
    assert!(output.status.success());
    let help = String::from_utf8(output.stdout).unwrap();
    assert!(help.contains("FLEETINT_AGENT_TOKEN"));
    assert!(!help.contains("example-private-token"));
}
