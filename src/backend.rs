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

//! Shared backend session/helper for the OOB agent runtime.
//!
//! Centralizes the agent's backend concerns that inventory and attestation both
//! depend on: API endpoint/URL resolution, the JWT/enrollment lifecycle, and
//! auth-failure detection. The typed request/response handling stays inside the
//! `bmc-inventory` and `bmc-attestation` crate sinks; this module only owns the
//! cross-cutting session state.

use std::error::Error;
use std::time::Duration;

pub use backend_client::is_auth_failure;
use backend_client::{BackendUrl, DEFAULT_BACKEND_TIMEOUT_SECONDS, http_client_builder};
use bmc_attestation::config as attestation_config;
use bmc_attestation::source::now_rfc3339;

/// Resolved backend endpoints derived from a single API base URL.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct BackendEndpoints {
    pub base_url: String,
    pub enroll_endpoint: String,
    pub oob_nodes_endpoint: String,
    pub nonce_path: String,
    pub attestation_path: String,
}

impl BackendEndpoints {
    /// Derive the agent endpoints from the API base URL. The base URL must not
    /// already include a `/v1/agent` path suffix.
    pub fn resolve(backend_url: &str) -> Result<Self, Box<dyn Error>> {
        let base_url = backend_url.trim().trim_end_matches('/').to_string();
        if base_url.ends_with("/v1/agent")
            || base_url.ends_with("/v1/agent/enroll")
            || base_url.ends_with("/v1/agent/oob_nodes")
        {
            return Err(
                "backend URL must be the API base URL without a /v1/agent path suffix".into(),
            );
        }

        let base = BackendUrl::parse(&base_url)?;
        Ok(Self {
            enroll_endpoint: base.append_endpoint("v1/agent/enroll").to_string(),
            oob_nodes_endpoint: base.append_endpoint("v1/agent/oob_nodes").to_string(),
            nonce_path: attestation_config::defaults::DEFAULT_BACKEND_NONCE_PATH.to_string(),
            attestation_path: attestation_config::defaults::DEFAULT_BACKEND_ATTESTATION_PATH
                .to_string(),
            base_url,
        })
    }
}

/// Require a backend CLI/env option to be present and non-empty.
pub fn required_backend_option(
    value: Option<String>,
    name: &'static str,
) -> Result<String, Box<dyn Error>> {
    value
        .filter(|value| !value.trim().is_empty())
        .ok_or_else(|| format!("{name} is required when running backend runtime").into())
}

/// Owns the agent's authenticated backend session: the current JWT and the
/// enrollment timestamp, plus the credentials needed to re-enroll.
pub struct BackendSession {
    agent_token: String,
    enroll_endpoint: String,
    jwt: String,
    enrolled_at: String,
    client: reqwest::Client,
}

impl BackendSession {
    /// Perform the initial enrollment and capture the issued JWT + timestamp.
    pub async fn enroll(
        enroll_endpoint: String,
        agent_token: String,
    ) -> Result<Self, Box<dyn Error>> {
        let client = http_client_builder(
            Duration::from_secs(DEFAULT_BACKEND_TIMEOUT_SECONDS),
            reqwest::redirect::Policy::default(),
        )?
        .build()?;
        let Enrollment { jwt, enrolled_at } =
            enroll_once(&client, &enroll_endpoint, agent_token.as_str()).await?;
        Ok(Self {
            agent_token,
            enroll_endpoint,
            jwt,
            enrolled_at,
            client,
        })
    }

    pub fn jwt(&self) -> &str {
        self.jwt.as_str()
    }

    pub fn enrolled_at(&self) -> &str {
        &self.enrolled_at
    }

    /// Update the JWT in place (e.g. when the backend rotates the token on a
    /// nonce response) without re-enrolling.
    pub fn set_jwt(&mut self, jwt: String) {
        self.jwt = jwt;
    }

    /// Re-enroll with the bootstrap token, refreshing both the JWT and the
    /// enrollment timestamp.
    pub async fn reenroll(&mut self) -> Result<(), Box<dyn Error>> {
        let Enrollment { jwt, enrolled_at } = enroll_once(
            &self.client,
            &self.enroll_endpoint,
            self.agent_token.as_str(),
        )
        .await?;
        self.jwt = jwt;
        self.enrolled_at = enrolled_at;
        Ok(())
    }
}

struct Enrollment {
    jwt: String,
    enrolled_at: String,
}

async fn enroll_once(
    client: &reqwest::Client,
    enroll_endpoint: &str,
    agent_token: &str,
) -> Result<Enrollment, Box<dyn Error>> {
    let endpoint = BackendUrl::parse(enroll_endpoint)?;
    let jwt = backend_client::enroll(client, endpoint.as_url(), agent_token).await?;
    Ok(Enrollment {
        jwt,
        enrolled_at: now_rfc3339(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn resolve_accepts_base_url() {
        let endpoints = BackendEndpoints::resolve("https://api.example.com/").expect("endpoints");
        assert_eq!(
            endpoints.enroll_endpoint,
            "https://api.example.com/v1/agent/enroll"
        );
        assert_eq!(
            endpoints.oob_nodes_endpoint,
            "https://api.example.com/v1/agent/oob_nodes"
        );
        assert_eq!(endpoints.base_url, "https://api.example.com");
    }

    #[test]
    fn resolve_trims_trailing_slashes() {
        let endpoints = BackendEndpoints::resolve("https://api.example.com///").expect("endpoints");
        assert_eq!(
            endpoints.enroll_endpoint,
            "https://api.example.com/v1/agent/enroll"
        );
        assert_eq!(
            endpoints.oob_nodes_endpoint,
            "https://api.example.com/v1/agent/oob_nodes"
        );
    }

    #[test]
    fn resolve_rejects_full_endpoint_urls() {
        let err = BackendEndpoints::resolve("https://api.example.com/v1/agent/enroll")
            .expect_err("error");
        assert!(err.to_string().contains("API base URL"));

        let err = BackendEndpoints::resolve("https://api.example.com/v1/agent/oob_nodes")
            .expect_err("error");
        assert!(err.to_string().contains("API base URL"));

        let err =
            BackendEndpoints::resolve("https://api.example.com/v1/agent/").expect_err("error");
        assert!(err.to_string().contains("API base URL"));
    }

    #[test]
    fn required_backend_option_rejects_blank_values() {
        assert!(required_backend_option(None::<String>, "backend URL").is_err());
        assert!(required_backend_option(Some("   ".to_string()), "agent token").is_err());
        assert_eq!(
            required_backend_option(Some("value".to_string()), "backend URL").expect("value"),
            "value"
        );
    }
}
