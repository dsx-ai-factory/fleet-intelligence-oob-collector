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
use std::fmt;
use std::time::Duration;

use serde::Deserialize;
use url::Url;

use crate::models::BatchAttestation;

const BACKEND_USER_AGENT: &str = "fleet-intelligence-oob-collector";

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
    async fn get_nonce(&mut self, node_uuid: &str) -> Result<NonceResponse, Box<dyn Error>>;
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
        let base_url = base_url.as_ref().parse::<Url>()?;
        let client = reqwest::Client::builder()
            .timeout(timeout)
            .connect_timeout(timeout.min(Duration::from_secs(10)))
            .redirect(reqwest::redirect::Policy::none())
            .build()?;
        Ok(Self {
            base_url,
            nonce_path: nonce_path.into(),
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

    async fn get_nonce(&mut self, node_uuid: &str) -> Result<NonceResponse, Box<dyn Error>> {
        if node_uuid.is_empty() {
            return Err("node_uuid cannot be empty".into());
        }
        if self.jwt.is_empty() {
            return Err("backend jwt cannot be empty".into());
        }
        let path = self.nonce_path.replace("{node_uuid}", node_uuid);
        let endpoint = self.endpoint(&path)?;
        tracing::debug!(%endpoint, node_uuid, method = "POST", "requesting attestation nonce");
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

    async fn submit_batch(&self, batch: &BatchAttestation) -> Result<(), Box<dyn Error>> {
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
        Ok(())
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
mod tests {
    use super::*;

    #[test]
    fn backend_http_client_sets_user_agent() {
        let client = HttpBackendClient::new(
            "https://backend.example.com",
            "/v1/oob/nodes/{node_uuid}/nonce",
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
            "/v1/oob/nodes/{node_uuid}/nonce",
            "/v1/oob/attestations",
            "jwt",
            Duration::from_secs(1),
        )
        .expect("client");

        assert_eq!(
            client
                .endpoint(&client.nonce_path.replace("{node_uuid}", "node-1"))
                .expect("url")
                .as_str(),
            "https://backend.example.com/v1/oob/nodes/node-1/nonce"
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
