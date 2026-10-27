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

use std::error::Error;
use std::time::Duration;

pub use backend_client::BackendHttpError;
use backend_client::{
    BackendUrl, SecretString, ensure_success, http_client_builder, request as backend_request,
};
use serde::Deserialize;
use url::Url;

use crate::models::BatchAttestation;

pub struct BackendSink<C = HttpBackendClient> {
    client: C,
}

#[derive(Clone)]
pub struct HttpBackendClient {
    base_url: BackendUrl,
    nonce_path: String,
    attestation_path: String,
    jwt: SecretString,
    client: reqwest::Client,
}

#[derive(Clone, Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct NonceResponse {
    pub nonce: String,
    pub jwt_assertion: Option<SecretString>,
    pub nonce_refresh_timestamp: Option<String>,
}

#[async_trait::async_trait(?Send)]
pub trait BackendClient {
    fn jwt(&self) -> &str;
    fn set_jwt(&mut self, jwt: String);
    async fn get_nonce(&mut self, bmc_mac: &str) -> Result<NonceResponse, Box<dyn Error>>;
    async fn submit_batch(&self, batch: &BatchAttestation) -> Result<(), Box<dyn Error>>;
}

impl BackendSink<HttpBackendClient> {
    pub fn new(
        base_url: impl AsRef<str>,
        nonce_path: impl Into<String>,
        attestation_path: impl Into<String>,
        jwt: impl Into<String>,
        timeout: Duration,
    ) -> Result<Self, Box<dyn Error>> {
        Ok(Self::with_client(HttpBackendClient::new(
            base_url,
            nonce_path,
            attestation_path,
            jwt,
            timeout,
        )?))
    }
}

impl<C> BackendSink<C> {
    pub fn with_client(client: C) -> Self {
        Self { client }
    }

    pub fn client(&self) -> &C {
        &self.client
    }
}

impl<C> BackendSink<C>
where
    C: BackendClient,
{
    pub async fn write(&self, batch: &BatchAttestation) -> Result<(), Box<dyn Error>> {
        self.client.submit_batch(batch).await
    }
}

impl HttpBackendClient {
    pub fn new(
        base_url: impl AsRef<str>,
        nonce_path: impl Into<String>,
        attestation_path: impl Into<String>,
        jwt: impl Into<String>,
        timeout: Duration,
    ) -> Result<Self, Box<dyn Error>> {
        if timeout.is_zero() {
            return Err("backend timeout must be greater than zero".into());
        }
        let nonce_path = nonce_path.into();
        if nonce_path.contains("{node_uuid}") {
            return Err(
                "legacy nonce path template `{node_uuid}` is no longer supported; replace it with `{bmc_mac}`"
                    .into(),
            );
        }
        let base_url = BackendUrl::parse(base_url.as_ref())?;
        let client = http_client_builder(timeout, reqwest::redirect::Policy::none())?.build()?;
        Ok(Self {
            base_url,
            nonce_path,
            attestation_path: attestation_path.into(),
            jwt: SecretString::from(jwt.into()),
            client,
        })
    }

    fn nonce_endpoint(&self, bmc_mac: &str) -> Result<Url, Box<dyn Error>> {
        if bmc_mac.is_empty() {
            return Err("bmc_mac cannot be empty".into());
        }
        Ok(self
            .base_url
            .root_endpoint(&self.nonce_path.replace("{bmc_mac}", bmc_mac)))
    }
}

#[async_trait::async_trait(?Send)]
impl BackendClient for HttpBackendClient {
    fn jwt(&self) -> &str {
        self.jwt.expose_secret()
    }

    fn set_jwt(&mut self, jwt: String) {
        self.jwt = jwt.into();
    }

    async fn get_nonce(&mut self, bmc_mac: &str) -> Result<NonceResponse, Box<dyn Error>> {
        if self.jwt.expose_secret().is_empty() {
            return Err("backend jwt cannot be empty".into());
        }
        let endpoint = self.nonce_endpoint(bmc_mac)?;
        tracing::debug!(%endpoint, bmc_mac, method = "POST", "requesting attestation nonce");
        let response = backend_request(&self.client, reqwest::Method::POST, &endpoint)
            .bearer_auth(self.jwt.expose_secret())
            .send()
            .await?;
        let response = ensure_success(response).await?;
        let response = response.json::<NonceResponse>().await?;
        tracing::debug!(
            nonce_refresh_timestamp = response.nonce_refresh_timestamp.as_deref(),
            jwt_rotated = response.jwt_assertion.is_some(),
            "received attestation nonce response"
        );
        if response.nonce.is_empty() {
            return Err("nonce response missing nonce".into());
        }
        if let Some(jwt) = &response.jwt_assertion
            && !jwt.expose_secret().is_empty()
            && jwt.expose_secret() != self.jwt.expose_secret()
        {
            self.jwt = jwt.clone();
        }
        Ok(response)
    }

    async fn submit_batch(&self, batch: &BatchAttestation) -> Result<(), Box<dyn Error>> {
        if self.jwt.expose_secret().is_empty() {
            return Err("backend jwt cannot be empty".into());
        }
        let endpoint = self.base_url.root_endpoint(&self.attestation_path);
        let request = batch.backend_request();
        tracing::info!(%endpoint, "exporting attestation data to backend");
        let successful_count = request
            .attestations
            .values()
            .filter(|attestation| attestation.success)
            .count();
        let evidence_count: usize = request
            .attestations
            .values()
            .map(|attestation| attestation.evidences.len())
            .sum();
        tracing::debug!(
            schema_version = %request.schema_version,
            node_count = request.attestations.len(),
            successful_count,
            failed_count = request.attestations.len().saturating_sub(successful_count),
            evidence_count,
            "attestation request prepared"
        );
        let response = backend_request(&self.client, reqwest::Method::POST, &endpoint)
            .bearer_auth(self.jwt.expose_secret())
            .json(&request)
            .send()
            .await?;
        ensure_success(response).await?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn nonce_response_debug_redacts_rotated_jwt() {
        let response: NonceResponse = serde_json::from_str(
            r#"{"nonce":"example-nonce","jwtAssertion":"example-private-jwt"}"#,
        )
        .unwrap();
        assert!(!format!("{response:?}").contains("example-private-jwt"));
        assert_eq!(
            response.jwt_assertion.unwrap().expose_secret(),
            "example-private-jwt"
        );
    }

    #[test]
    fn backend_http_client_sets_user_agent() {
        let client = HttpBackendClient::new(
            "https://backend.example.com",
            "/v1/oob/nodes/{bmc_mac}/nonce",
            "/v1/oob/attestations",
            "jwt",
            Duration::from_secs(1),
        )
        .expect("client");
        let request = backend_request(
            &client.client,
            reqwest::Method::POST,
            &"https://backend.example.com/v1/oob/attestations"
                .parse()
                .expect("endpoint"),
        )
        .build()
        .expect("request");

        assert_eq!(
            request.headers()[reqwest::header::USER_AGENT],
            backend_client::BACKEND_USER_AGENT
        );
    }

    #[test]
    fn builds_backend_endpoint() {
        let client = HttpBackendClient::new(
            "https://backend.example.com/base",
            "/v1/oob/nodes/{bmc_mac}/nonce",
            "/v1/oob/attestations",
            "jwt",
            Duration::from_secs(1),
        )
        .expect("client");

        assert_eq!(
            client
                .nonce_endpoint("00:11:22:33:44:55")
                .expect("url")
                .as_str(),
            "https://backend.example.com/v1/oob/nodes/00:11:22:33:44:55/nonce"
        );
    }

    #[test]
    fn rejects_empty_bmc_mac_for_nonce_endpoint() {
        let client = HttpBackendClient::new(
            "https://backend.example.com",
            "/v1/oob/nodes/{bmc_mac}/nonce",
            "/v1/oob/attestations",
            "jwt",
            Duration::from_secs(1),
        )
        .expect("client");

        assert_eq!(
            client
                .nonce_endpoint("")
                .expect_err("empty BMC MAC should fail")
                .to_string(),
            "bmc_mac cannot be empty"
        );
    }

    #[test]
    fn rejects_legacy_node_uuid_nonce_path_template() {
        let error = HttpBackendClient::new(
            "https://backend.example.com",
            "/v1/oob/nodes/{node_uuid}/nonce",
            "/v1/oob/attestations",
            "jwt",
            Duration::from_secs(1),
        )
        .err()
        .expect("legacy node UUID template should fail");

        assert_eq!(
            error.to_string(),
            "legacy nonce path template `{node_uuid}` is no longer supported; replace it with `{bmc_mac}`"
        );
    }
}
