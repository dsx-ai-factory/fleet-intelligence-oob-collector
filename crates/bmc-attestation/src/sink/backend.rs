use std::error::Error;
use std::fmt;
use std::time::Duration;

use serde::Deserialize;
use url::Url;

use crate::models::BatchAttestation;

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
        tracing::info!(%endpoint, node_uuid, "requesting attestation nonce from backend");
        if tracing::enabled!(tracing::Level::DEBUG) {
            let request = serde_json::json!({
                "method": "POST",
                "url": endpoint.as_str(),
                "headers": {
                    "authorization": "Bearer <redacted>"
                },
                "body": null
            });
            tracing::debug!(request = %serde_json::to_string_pretty(&request)?, "nonce request");
        }
        let response = self
            .client
            .post(endpoint)
            .bearer_auth(&self.jwt)
            .send()
            .await?;
        let status = response.status();
        if !status.is_success() {
            let body = response.text().await.unwrap_or_default();
            return Err(Box::new(BackendHttpError::new(status.as_u16(), body)));
        }
        let response = response.json::<NonceResponse>().await?;
        tracing::info!(
            nonce_refresh_timestamp = response.nonce_refresh_timestamp.as_deref(),
            jwt_rotated = response.jwt_assertion.is_some(),
            "received attestation nonce response"
        );
        if response.nonce.is_empty() {
            return Err("nonce response missing nonce".into());
        }
        if let Some(jwt) = &response.jwt_assertion {
            if !jwt.is_empty() && jwt != &self.jwt {
                self.jwt = jwt.clone();
            }
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
        if tracing::enabled!(tracing::Level::DEBUG) {
            tracing::debug!(request = %serde_json::to_string_pretty(&request)?, "attestation request body");
        }
        let response = self
            .client
            .post(endpoint)
            .bearer_auth(&self.jwt)
            .json(&request)
            .send()
            .await?;
        let status = response.status();
        if !status.is_success() {
            let body = response.text().await.unwrap_or_default();
            return Err(Box::new(BackendHttpError::new(status.as_u16(), body)));
        }
        Ok(())
    }
}

/// A non-success HTTP response from the attestation backend, preserving the
/// status code so callers can distinguish JWT rejections (401/403) and re-enroll.
#[derive(Debug)]
pub struct BackendHttpError {
    status: u16,
    body: String,
}

impl BackendHttpError {
    pub fn new(status: u16, body: impl Into<String>) -> Self {
        Self {
            status,
            body: body.into(),
        }
    }

    pub fn status(&self) -> u16 {
        self.status
    }
}

impl fmt::Display for BackendHttpError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        if self.body.trim().is_empty() {
            write!(f, "backend request failed with HTTP {}", self.status)
        } else {
            write!(
                f,
                "backend request failed with HTTP {}: {}",
                self.status, self.body
            )
        }
    }
}

impl Error for BackendHttpError {}

#[cfg(test)]
mod tests {
    use super::*;

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
}
