// Copyright (c) 2026, NVIDIA CORPORATION & AFFILIATES. All rights reserved.
// SPDX-License-Identifier: Apache-2.0

//! List source endpoints without loading BMC credentials or collecting hardware.

use std::error::Error;
use std::io::Write;

#[tokio::main]
async fn main() -> Result<(), Box<dyn Error>> {
    let path = std::env::args()
        .nth(1)
        .unwrap_or_else(|| "config.toml".into());
    let config = bmc_inventory::config::load_source_config(&path).await?;
    let endpoints = config
        .source
        .discover()
        .await
        .map_err(|error| error as Box<dyn Error>)?;
    let output: Vec<_> = endpoints
        .iter()
        .map(|endpoint| {
            serde_json::json!({
                "ip": endpoint.ip,
                "mac": endpoint.mac.to_string().to_uppercase(),
                "name": endpoint.name,
                "kind": endpoint.kind,
            })
        })
        .collect();
    serde_json::to_writer_pretty(std::io::stdout().lock(), &output)?;
    std::io::stdout().write_all(b"\n")?;
    Ok(())
}
