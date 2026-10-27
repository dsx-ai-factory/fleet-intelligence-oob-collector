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

use std::collections::BTreeMap;
use std::error::Error;
use std::fmt;
use std::time::Duration;

use bmc_error::SanitizedErrorMessage;
use mac_address::MacAddress;
use serde::Deserialize;
use url::Url;

use crate::models::BatchAttestation;

const BACKEND_USER_AGENT: &str = "fleet-intelligence-oob-collector";

#[derive(Deserialize)]
struct BatchResponse {
    results: Vec<BatchItemResult>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct BatchItemResult {
    bmc_mac: Option<String>,
    status: String,
}

#[derive(Debug, PartialEq, Eq)]
pub enum TargetUploadOutcome {
    Accepted,
    Rejected(SanitizedErrorMessage),
    Missing,
    Duplicate,
}

#[derive(Debug)]
pub struct BatchUploadOutcome {
    pub targets: BTreeMap<String, TargetUploadOutcome>,
    pub unexpected_results: usize,
}

impl BatchResponse {
    /// Preserve each target's outcome even when other response rows are invalid.
    fn outcomes(self, batch: &BatchAttestation) -> BatchUploadOutcome {
        let mut outcome = BatchUploadOutcome {
            targets: batch
                .results_by_bmc_mac
                .keys()
                .map(|mac| (mac.clone(), TargetUploadOutcome::Missing))
                .collect(),
            unexpected_results: 0,
        };
        for item in self.results {
            let Some(result) = item
                .bmc_mac
                .as_deref()
                .and_then(|mac| mac.trim().parse::<MacAddress>().ok())
                .and_then(|mac| outcome.targets.get_mut(&mac.to_string()))
            else {
                // Do not echo arbitrary backend-provided identifiers or messages.
                outcome.unexpected_results += 1;
                continue;
            };
            *result = if *result != TargetUploadOutcome::Missing {
                TargetUploadOutcome::Duplicate
            } else if item.status.eq_ignore_ascii_case("OK") {
                TargetUploadOutcome::Accepted
            } else {
                TargetUploadOutcome::Rejected(SanitizedErrorMessage::new(item.status))
            };
        }
        outcome
    }
}

pub struct BackendSink<C = HttpBackendClient> {
    client: C,
}

#[derive(Clone)]
pub struct HttpBackendClient {
    base_url: Url,
    nonce_path: String,
    attestation_path: String,
    jwt: String,
    client: reqwest::Client,
}

#[derive(Clone, Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct NonceResponse {
    pub nonce: String,
    pub jwt_assertion: Option<String>,
    pub nonce_refresh_timestamp: Option<String>,
}

#[async_trait::async_trait(?Send)]
pub trait BackendClient {
    fn jwt(&self) -> &str;
    fn set_jwt(&mut self, jwt: String);
    async fn get_nonce(&mut self, bmc_mac: &str) -> Result<NonceResponse, Box<dyn Error>>;
    async fn submit_batch(
        &self,
        batch: &BatchAttestation,
    ) -> Result<BatchUploadOutcome, Box<dyn Error>>;
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
    pub async fn write(
        &self,
        batch: &BatchAttestation,
    ) -> Result<BatchUploadOutcome, Box<dyn Error>> {
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
        let base_url = base_url.as_ref().parse::<Url>()?;
        let client = reqwest::Client::builder()
            .timeout(timeout)
            .connect_timeout(timeout.min(Duration::from_secs(10)))
            .redirect(reqwest::redirect::Policy::none())
            .build()?;
        Ok(Self {
            base_url,
            nonce_path,
            attestation_path: attestation_path.into(),
            jwt: jwt.into(),
            client,
        })
    }

    fn endpoint(&self, path: &str) -> Result<Url, Box<dyn Error>> {
        let mut url = self.base_url.clone();
        let mut segments = url
            .path_segments_mut()
            .map_err(|()| "backend base URL cannot be a base")?;
        segments.clear();
        for segment in path.trim_start_matches('/').split('/') {
            if !segment.is_empty() {
                segments.push(segment);
            }
        }
        drop(segments);
        Ok(url)
    }

    fn nonce_endpoint(&self, bmc_mac: &str) -> Result<Url, Box<dyn Error>> {
        if bmc_mac.is_empty() {
            return Err("bmc_mac cannot be empty".into());
        }
        self.endpoint(&self.nonce_path.replace("{bmc_mac}", bmc_mac))
    }

    fn request(&self, method: reqwest::Method, endpoint: Url) -> reqwest::RequestBuilder {
        self.client
            .request(method, endpoint)
            .header(reqwest::header::USER_AGENT, BACKEND_USER_AGENT)
    }
}

#[async_trait::async_trait(?Send)]
impl BackendClient for HttpBackendClient {
    fn jwt(&self) -> &str {
        &self.jwt
    }

    fn set_jwt(&mut self, jwt: String) {
        self.jwt = jwt;
    }

    async fn get_nonce(&mut self, bmc_mac: &str) -> Result<NonceResponse, Box<dyn Error>> {
        if self.jwt.is_empty() {
            return Err("backend jwt cannot be empty".into());
        }
        let endpoint = self.nonce_endpoint(bmc_mac)?;
        tracing::debug!(%endpoint, bmc_mac, method = "POST", "requesting attestation nonce");
        let response = self
            .request(reqwest::Method::POST, endpoint)
            .bearer_auth(&self.jwt)
            .send()
            .await?;
        let status = response.status();
        if !status.is_success() {
            let server = response_header(&response, reqwest::header::SERVER.as_str());
            let request_id = response_header(&response, "x-request-id");
            let body = response.text().await.unwrap_or_default();
            return Err(Box::new(BackendHttpError::with_metadata(
                status.as_u16(),
                body,
                server,
                request_id,
            )));
        }
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
            && !jwt.is_empty()
            && jwt != &self.jwt
        {
            self.jwt = jwt.clone();
        }
        Ok(response)
    }

    async fn submit_batch(
        &self,
        batch: &BatchAttestation,
    ) -> Result<BatchUploadOutcome, Box<dyn Error>> {
        if self.jwt.is_empty() {
            return Err("backend jwt cannot be empty".into());
        }
        let endpoint = self.endpoint(&self.attestation_path)?;
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
        let response = self
            .request(reqwest::Method::POST, endpoint)
            .bearer_auth(&self.jwt)
            .json(&request)
            .send()
            .await?;
        let status = response.status();
        if !status.is_success() {
            let server = response_header(&response, reqwest::header::SERVER.as_str());
            let request_id = response_header(&response, "x-request-id");
            let body = response.text().await.unwrap_or_default();
            return Err(Box::new(BackendHttpError::with_metadata(
                status.as_u16(),
                body,
                server,
                request_id,
            )));
        }
        Ok(response.json::<BatchResponse>().await?.outcomes(batch))
    }
}

fn response_header(response: &reqwest::Response, name: &str) -> Option<String> {
    response
        .headers()
        .get(name)
        .and_then(|value| value.to_str().ok())
        .map(str::to_string)
}

/// A non-success HTTP response from the attestation backend, preserving the
/// status code so callers can distinguish JWT rejections and re-enroll.
#[derive(Debug)]
pub struct BackendHttpError {
    status: u16,
    server: Option<String>,
    request_id: Option<String>,
    body: String,
}

impl BackendHttpError {
    pub fn new(status: u16, body: impl Into<String>) -> Self {
        Self::with_metadata(status, body, None, None)
    }

    fn with_metadata(
        status: u16,
        body: impl Into<String>,
        server: Option<String>,
        request_id: Option<String>,
    ) -> Self {
        let body = body.into();
        let body = body
            .chars()
            .map(|character| {
                if character.is_control() {
                    ' '
                } else {
                    character
                }
            })
            .collect();
        Self {
            status,
            server,
            request_id,
            body,
        }
    }

    pub fn status(&self) -> u16 {
        self.status
    }
}

impl fmt::Display for BackendHttpError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "backend request failed http_status={}", self.status)?;
        if let Some(server) = &self.server {
            write!(f, " server={server:?}")?;
        }
        if let Some(request_id) = &self.request_id {
            write!(f, " request_id={request_id:?}")?;
        }
        if !self.body.trim().is_empty() {
            write!(f, " response_body={:?}", self.body)?;
        }
        Ok(())
    }
}

impl Error for BackendHttpError {}

#[cfg(test)]
#[path = "backend_tests.rs"]
mod upload_tests;

#[cfg(test)]
mod tests {
    use super::*;

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
        let request = client
            .request(
                reqwest::Method::POST,
                "https://backend.example.com/v1/oob/attestations"
                    .parse()
                    .expect("endpoint"),
            )
            .build()
            .expect("request");

        assert_eq!(
            request.headers()[reqwest::header::USER_AGENT],
            BACKEND_USER_AGENT
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

    #[test]
    fn backend_error_includes_full_body_and_response_metadata() {
        let body = "first line\nsecond line".to_string();
        let error = BackendHttpError::with_metadata(
            403,
            body.clone(),
            Some("awselb/2.0".to_string()),
            Some("request-123".to_string()),
        );
        let message = error.to_string();

        assert!(message.contains("http_status=403"));
        assert!(message.contains("server=\"awselb/2.0\""));
        assert!(message.contains("request_id=\"request-123\""));
        assert!(message.contains("response_body=\"first line second line\""));
        assert!(!message.contains('\n'));
    }
}
