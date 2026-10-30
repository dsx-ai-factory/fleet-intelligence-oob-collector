// Copyright (c) 2026, NVIDIA CORPORATION & AFFILIATES. All rights reserved.
// SPDX-License-Identifier: Apache-2.0

//! Service settings shared by configuration rendering and backend reporting.

use std::error::Error;
use std::num::NonZeroU32;

use serde::{Deserialize, Serialize};

/// Configured NICo overrides. Omitted values use the bundled NICo defaults.
#[derive(Clone, Debug, Default, Deserialize, Serialize)]
#[serde(default, deny_unknown_fields, rename_all(serialize = "camelCase"))]
pub struct NicoHealthSettings {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub sensor_fetch_interval_seconds: Option<NonZeroU32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub bmc_request_concurrency: Option<NonZeroU32>,
}

impl NicoHealthSettings {
    pub(super) fn validate(&self) -> Result<(), Box<dyn Error>> {
        if let Some(concurrency) = self.bmc_request_concurrency
            && u64::from(concurrency.get()) > tokio::sync::Semaphore::MAX_PERMITS as u64
        {
            return Err(
                "nico_health.bmc_request_concurrency exceeds NICo's semaphore limit".into(),
            );
        }
        Ok(())
    }
}
