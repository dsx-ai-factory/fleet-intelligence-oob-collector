// Copyright (c) 2025-2026, NVIDIA CORPORATION & AFFILIATES. All rights reserved.
//
// Licensed under the Apache License, Version 2.0 (the "License");
// you may not use this file except in compliance with the License.
// You may obtain a copy of the License at
//
//     http://www.apache.org/licenses/LICENSE-2.0
//
// Unless required by applicable law or agreed to in writing, software
// distributed under the License is distributed on an "AS IS" BASIS,
// WITHOUT WARRANTIES OR CONDITIONS OF ANY KIND, either express or implied.
// See the License for the specific language governing permissions and
// limitations under the License.
//
// SPDX-License-Identifier: Apache-2.0

use bmc_error::SanitizedErrorMessage;
use serde::Serialize;

/// How collector-to-BMC latency was measured.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum LatencyMethod {
    /// Time to establish a TCP connection to the Redfish port; no TLS or HTTP.
    TcpConnect,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum LatencyStatus {
    /// Enough attempts succeeded to report a round minimum.
    Ok,
    /// Too few attempts succeeded; the round carries no latency value.
    Unavailable,
    /// A local component answered TCP for an unroutable address, so connect
    /// timing does not reflect the BMC path and the round was skipped.
    Intercepted,
    /// The probe is turned off; no probe traffic was sent.
    Disabled,
}

/// One measurement round from this collector to one BMC.
#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct LatencyMeasurement {
    pub method: LatencyMethod,
    pub status: LatencyStatus,
    pub destination: String,
    pub measured_at: String,
    pub attempts: u32,
    pub successes: u32,
    pub min_us: Option<u64>,
    pub median_us: Option<u64>,
    pub error: Option<SanitizedErrorMessage>,
}
