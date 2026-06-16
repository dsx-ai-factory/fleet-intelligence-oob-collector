use std::error::Error;
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
        let request = serde_json::json!({
            "method": "POST",
            "url": endpoint.as_str(),
            "headers": {
                "authorization": "Bearer <redacted>"
            },
            "body": null
        });
        println!("Requesting attestation nonce from backend:");
        println!("{}", serde_json::to_string_pretty(&request)?);
        let response = self
            .client
            .post(endpoint)
            .bearer_auth(&self.jwt)
            .send()
            .await?
            .error_for_status()?
            .json::<NonceResponse>()
            .await?;
        let response_log = serde_json::json!({
            "nonce": &response.nonce,
            "jwtAssertion": response.jwt_assertion.as_ref().map(|_| "<redacted>"),
            "nonceRefreshTimestamp": &response.nonce_refresh_timestamp
        });
        println!("Received attestation nonce response:");
        println!("{}", serde_json::to_string_pretty(&response_log)?);
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
        println!("Exporting attestation data to backend: {endpoint}");
        println!("{}", serde_json::to_string_pretty(&request)?);
        self.client
            .post(endpoint)
            .bearer_auth(&self.jwt)
            .json(&request)
            .send()
            .await?
            .error_for_status()?;
        Ok(())
    }
}

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
