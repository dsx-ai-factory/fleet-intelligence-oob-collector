// Copyright (c) 2026, NVIDIA CORPORATION & AFFILIATES. All rights reserved.
// SPDX-License-Identifier: Apache-2.0

//! List source endpoints without loading BMC credentials or collecting hardware.

use std::error::Error;

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
    bmc_inventory::endpoint::write_discovery_json(&endpoints, std::io::stdout().lock())?;
    Ok(())
}
