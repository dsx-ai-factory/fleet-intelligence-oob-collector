// Copyright (c) 2026, NVIDIA CORPORATION & AFFILIATES. All rights reserved.
// SPDX-License-Identifier: Apache-2.0

//! Shared backend HTTP policy, enrollment transport, and typed errors. Inventory and
//! attestation retain ownership of their request and response payloads.

use std::error::Error;
use std::fmt;
use std::time::Duration;

use bmc_error::SanitizedErrorMessage;
use reqwest::{Client, Method, RequestBuilder, Response};
use url::Url;

pub const DEFAULT_BACKEND_TIMEOUT_SECONDS: u64 = 30;
pub const DEFAULT_BACKEND_CONNECT_TIMEOUT_SECONDS: u64 = 10;
pub const BACKEND_USER_AGENT: &str = "fleet-intelligence-oob-collector";

#[derive(Clone, Debug)]
pub struct BackendUrl(Url);

impl BackendUrl {
    pub fn parse(value: &str) -> Result<Self, Box<dyn Error>> {
        let url = Url::parse(value.trim()).map_err(|_| "invalid backend URL")?;
        if !matches!(url.scheme(), "http" | "https") || url.host_str().is_none() {
            return Err("backend URL must be an absolute HTTP or HTTPS URL".into());
        }
        if !url.username().is_empty() || url.password().is_some() {
            return Err("backend URL must not contain credentials".into());
        }
        if url.query().is_some() || url.fragment().is_some() {
            return Err("backend URL must not contain a query or fragment".into());
        }
        Ok(Self(url))
    }

    pub fn as_url(&self) -> &Url {
        &self.0
    }

    pub fn append_endpoint(&self, path: &str) -> Url {
        let mut url = self.0.clone();
        // Validation guarantees a hierarchical HTTP(S) URL.
        let mut segments = url.path_segments_mut().expect("validated HTTP URL");
        segments.pop_if_empty();
        for segment in path.split('/').filter(|segment| !segment.is_empty()) {
            segments.push(segment);
        }
        drop(segments);
        url
    }

    pub fn root_endpoint(&self, path: &str) -> Url {
        let mut root = self.clone();
        root.0.set_path("/");
        root.append_endpoint(path)
    }
}

pub fn http_client_builder(
    timeout: Duration,
    redirects: reqwest::redirect::Policy,
) -> Result<reqwest::ClientBuilder, Box<dyn Error>> {
    if timeout.is_zero() {
        return Err("backend request timeout must be greater than zero".into());
    }
    Ok(Client::builder()
        .timeout(timeout)
        .connect_timeout(timeout.min(Duration::from_secs(DEFAULT_BACKEND_CONNECT_TIMEOUT_SECONDS)))
        .redirect(redirects))
}

pub fn request(client: &Client, method: Method, endpoint: &Url) -> RequestBuilder {
    client
        .request(method, endpoint.clone())
        .header(reqwest::header::USER_AGENT, BACKEND_USER_AGENT)
}

pub async fn ensure_success(response: Response) -> Result<Response, Box<dyn Error>> {
    let status = response.status();
    if status.is_success() {
        return Ok(response);
    }
    let header = |name: &str| {
        response
            .headers()
            .get(name)
            .and_then(|value| value.to_str().ok())
            .map(str::to_string)
    };
    let server = header(reqwest::header::SERVER.as_str());
    let request_id = header("x-request-id");
    let body = response.text().await.unwrap_or_default();
    Err(Box::new(BackendHttpError::with_metadata(
        status.as_u16(),
        body,
        server,
        request_id,
    )))
}

pub async fn enroll(
    client: &Client,
    endpoint: &Url,
    token: &str,
) -> Result<String, Box<dyn Error>> {
    #[derive(serde::Deserialize)]
    #[serde(rename_all = "camelCase")]
    struct EnrollResponse {
        jwt_assertion: String,
    }
    if token.trim().is_empty() {
        return Err("agent token cannot be empty".into());
    }
    let response = request(client, Method::POST, endpoint)
        .bearer_auth(token)
        .send()
        .await?;
    let response: EnrollResponse = ensure_success(response).await?.json().await?;
    if response.jwt_assertion.trim().is_empty() {
        return Err("enrollment response missing jwtAssertion field".into());
    }
    Ok(response.jwt_assertion)
}

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
        let body = SanitizedErrorMessage::new(body.into())
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
            server: server.map(|value| SanitizedErrorMessage::new(value).to_string()),
            request_id: request_id.map(|value| SanitizedErrorMessage::new(value).to_string()),
            body,
        }
    }

    pub fn status(&self) -> u16 {
        self.status
    }
}

impl fmt::Display for BackendHttpError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            formatter,
            "backend request failed http_status={}",
            self.status
        )?;
        if let Some(server) = &self.server {
            write!(formatter, " server={server:?}")?;
        }
        if let Some(request_id) = &self.request_id {
            write!(formatter, " request_id={request_id:?}")?;
        }
        if !self.body.trim().is_empty() {
            write!(formatter, " response_body={:?}", self.body)?;
        }
        Ok(())
    }
}

impl Error for BackendHttpError {}

/// Only a typed HTTP 401 is evidence that the backend rejected the JWT.
pub fn is_auth_failure(error: &(dyn Error + 'static)) -> bool {
    error
        .downcast_ref::<BackendHttpError>()
        .is_some_and(|error| error.status() == 401)
}

#[cfg(test)]
mod tests {
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    use tokio::net::TcpListener;

    use super::*;

    #[test]
    fn url_validation_rejects_ambiguous_or_sensitive_urls_without_echoing_them() {
        for input in [
            "relative/path",
            "ftp://example.com",
            "https://user:example-secret@example.com",
            "https://example.com?token=example-secret",
            "https://example.com#example-secret",
        ] {
            let error = BackendUrl::parse(input).expect_err(input).to_string();
            assert!(!error.contains("example-secret"));
            assert!(!error.contains(input));
        }
        for input in [
            "https://api.example.com",
            "http://127.0.0.1:8000",
            "https://[::1]:8443/prefix/",
        ] {
            BackendUrl::parse(input).expect("valid backend URL");
        }
    }

    #[test]
    fn backend_paths_preserve_existing_base_prefix_behavior() {
        let base = BackendUrl::parse("https://example.com/prefix/").unwrap();
        assert_eq!(
            base.append_endpoint("/v1/agent/enroll").as_str(),
            "https://example.com/prefix/v1/agent/enroll"
        );
        assert_eq!(
            base.root_endpoint("/v1/agent/oob_nodes/AA:BB:CC:DD:EE:FF/nonce")
                .as_str(),
            "https://example.com/v1/agent/oob_nodes/AA:BB:CC:DD:EE:FF/nonce"
        );
    }

    #[test]
    fn only_http_unauthorized_is_an_auth_failure() {
        for status in [400, 401, 403, 429, 500] {
            assert_eq!(
                is_auth_failure(&BackendHttpError::new(status, "example")),
                status == 401
            );
        }
        let text: Box<dyn Error> = "HTTP 401 is not a typed response".into();
        assert!(!is_auth_failure(text.as_ref()));
    }

    #[test]
    fn error_metadata_preserves_diagnostics_and_sanitizes_urls() {
        let error = BackendHttpError::with_metadata(
            403,
            "first line\nhttps://user:example-secret@example.com/path?token=hidden",
            Some("example-proxy".into()),
            Some("request-123".into()),
        );
        let message = error.to_string();
        assert!(message.contains("http_status=403"));
        assert!(message.contains("request-123"));
        assert!(message.contains("first line"));
        assert!(message.contains("URL_REDACTED"));
        assert!(!message.contains("example-secret"));
        assert!(!message.contains('\n'));
    }

    /// Local HTTP fixture for response parsing and typed error handling.
    async fn fixture(responses: Vec<String>) -> (Url, tokio::task::JoinHandle<()>) {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let endpoint = format!("http://{}/enroll", listener.local_addr().unwrap())
            .parse()
            .unwrap();
        let server = tokio::spawn(async move {
            for response in responses {
                let (mut socket, _) =
                    tokio::time::timeout(Duration::from_secs(3), listener.accept())
                        .await
                        .unwrap()
                        .unwrap();
                let mut bytes = Vec::new();
                loop {
                    let mut part = [0; 4096];
                    let count = socket.read(&mut part).await.unwrap();
                    assert!(count > 0);
                    bytes.extend_from_slice(&part[..count]);
                    if bytes.windows(4).any(|value| value == b"\r\n\r\n") {
                        break;
                    }
                }
                socket.write_all(response.as_bytes()).await.unwrap();
            }
        });
        (endpoint, server)
    }

    fn response(status: u16, headers: &str, body: &str) -> String {
        format!(
            "HTTP/1.1 {status} Test\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n{headers}\r\n{body}",
            body.len()
        )
    }

    fn local_client(policy: reqwest::redirect::Policy) -> Client {
        http_client_builder(Duration::from_secs(1), policy)
            .unwrap()
            .no_proxy()
            .build()
            .unwrap()
    }

    #[tokio::test]
    async fn response_status_and_metadata_remain_typed_over_http() {
        for status in [401, 403, 500] {
            let (endpoint, server) = fixture(vec![response(
                status,
                "Server: example-proxy\r\nx-request-id: example-request\r\n",
                "rejected",
            )])
            .await;
            let reply = request(
                &local_client(reqwest::redirect::Policy::none()),
                Method::GET,
                &endpoint,
            )
            .send()
            .await
            .unwrap();
            let error = ensure_success(reply)
                .await
                .expect_err("rejected HTTP status");
            assert_eq!(is_auth_failure(error.as_ref()), status == 401);
            assert_eq!(
                error.downcast_ref::<BackendHttpError>().unwrap().status(),
                status
            );
            assert!(error.to_string().contains("example-request"));
            server.await.unwrap();
        }
    }

    #[tokio::test]
    async fn enrollment_rejects_empty_jwt_and_unusable_responses() {
        for body in [r#"{"jwtAssertion":" "}"#, r#"{}"#, "not-json"] {
            let (endpoint, server) = fixture(vec![response(200, "", body)]).await;
            assert!(
                enroll(
                    &local_client(reqwest::redirect::Policy::default()),
                    &endpoint,
                    "example-token"
                )
                .await
                .is_err()
            );
            server.await.unwrap();
        }
        assert!(http_client_builder(Duration::ZERO, reqwest::redirect::Policy::default()).is_err());
    }
}
