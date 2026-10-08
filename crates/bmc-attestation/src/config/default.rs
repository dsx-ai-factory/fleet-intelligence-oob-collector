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

use std::time::Duration;

use super::AttestationConfig;

pub const DEFAULT_CONFIG_PATH: &str = ".config";
pub const DEFAULT_BACKEND_ATTESTATION_PATH: &str = "/v1/agent/oob_nodes/attestation";
pub const DEFAULT_BACKEND_NONCE_PATH: &str = "/v1/agent/oob_nodes/{node_uuid}/nonce";

pub const DEFAULT_PORT: u16 = 443;
pub const DEFAULT_INSECURE_TLS: bool = false;
pub const DEFAULT_CONCURRENCY: usize = 16;
pub const DEFAULT_REDFISH_TIMEOUT: Duration = Duration::from_secs(30);
pub const DEFAULT_TASK_TIMEOUT: Duration = Duration::from_secs(30);
pub const DEFAULT_TASK_POLL_INTERVAL: Duration = Duration::from_millis(1000);
pub const DEFAULT_INTERVAL: Option<Duration> = None;
pub const DEFAULT_RETRY_INTERVAL: Duration = Duration::from_secs(300);
pub const DEFAULT_BATCH_TIMEOUT: Option<Duration> = None;
pub const DEFAULT_MEASUREMENT_INDICES: &[u8] = &[255];

pub fn attestation_config() -> AttestationConfig {
    AttestationConfig {
        port: DEFAULT_PORT,
        insecure_tls: DEFAULT_INSECURE_TLS,
        concurrency: DEFAULT_CONCURRENCY,
        redfish_timeout: DEFAULT_REDFISH_TIMEOUT,
        task_timeout: DEFAULT_TASK_TIMEOUT,
        task_poll_interval: DEFAULT_TASK_POLL_INTERVAL,
        interval: DEFAULT_INTERVAL,
        retry_interval: DEFAULT_RETRY_INTERVAL,
        batch_timeout: DEFAULT_BATCH_TIMEOUT,
        measurement_indices: DEFAULT_MEASUREMENT_INDICES.to_vec(),
    }
}
