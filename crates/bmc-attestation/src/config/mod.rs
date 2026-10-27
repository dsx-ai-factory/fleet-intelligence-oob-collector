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

use backend_client::SecretString;

#[path = "default.rs"]
pub mod defaults;

#[derive(Clone, Debug)]
pub struct AttestationConfig {
    pub port: u16,
    pub insecure_tls: bool,
    pub concurrency: usize,
    pub redfish_timeout: Duration,
    pub task_timeout: Duration,
    pub task_poll_interval: Duration,
    pub interval: Option<Duration>,
    pub retry_interval: Duration,
    pub batch_timeout: Option<Duration>,
    pub measurement_indices: Vec<u8>,
}

#[derive(Clone, Debug)]
pub struct Target {
    pub name: String,
    pub bmc_mac: String,
    pub address: String,
    pub username: String,
    pub password: SecretString,
}

impl Default for AttestationConfig {
    fn default() -> Self {
        defaults::attestation_config()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn target_debug_redacts_passwords_after_cloning() {
        let target = Target {
            name: "bmc-1".into(),
            bmc_mac: "00:11:22:33:44:55".into(),
            address: "192.0.2.1".into(),
            username: "example-user".into(),
            password: "example-private-password".into(),
        };
        assert!(!format!("{:?}", target.clone()).contains("example-private-password"));
        assert_eq!(target.password.expose_secret(), "example-private-password");
    }
}
