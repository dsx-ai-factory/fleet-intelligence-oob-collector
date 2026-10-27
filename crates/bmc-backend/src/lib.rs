// Copyright (c) 2026, NVIDIA CORPORATION & AFFILIATES. All rights reserved.
// SPDX-License-Identifier: Apache-2.0

//! Shared backend HTTP policy and credential primitives. Inventory and
//! attestation retain ownership of their request and response payloads.

mod secret;

use std::error::Error;
use std::fmt;
use std::time::Duration;

use bmc_error::SanitizedErrorMessage;
use reqwest::{Client, Method, RequestBuilder, Response};
use url::Url;

pub use secret::SecretString;

pub const DEFAULT_BACKEND_TIMEOUT_SECONDS: u64 = 30;
pub const DEFAULT_BACKEND_CONNECT_TIMEOUT_SECONDS: u64 = 10;
pub const BACKEND_USER_AGENT: &str = "fleet-intelligence-oob-collector";

/// Explicit per-workflow redirect policy, preserving existing behavior.
#[derive(Clone, Copy)]
pub enum RedirectPolicy {
    Default,
    None,
}

/// Whether an API path extends or replaces the configured base path.
#[derive(Clone, Copy)]
pub enum PathMode {
    Append,
    Replace,
}

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

    pub fn endpoint(&self, path: &str, mode: PathMode) -> Url {
        let mut url = self.0.clone();
        // Validation guarantees a hierarchical HTTP(S) URL.
        let mut segments = url.path_segments_mut().expect("validated HTTP URL");
        match mode {
            PathMode::Append => {
                segments.pop_if_empty();
            }
            PathMode::Replace => {
                segments.clear();
            }
        }
        for segment in path.split('/').filter(|segment| !segment.is_empty()) {
            segments.push(segment);
        }
        drop(segments);
        url
    }
}

pub fn http_client(timeout: Duration, redirects: RedirectPolicy) -> Result<Client, Box<dyn Error>> {
    Ok(client_builder(timeout, redirects)?.build()?)
}

fn client_builder(
    timeout: Duration,
    redirects: RedirectPolicy,
) -> Result<reqwest::ClientBuilder, Box<dyn Error>> {
    if timeout.is_zero() {
        return Err("backend request timeout must be greater than zero".into());
    }
    let redirects = match redirects {
        RedirectPolicy::Default => reqwest::redirect::Policy::default(),
        RedirectPolicy::None => reqwest::redirect::Policy::none(),
    };
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
) -> Result<SecretString, Box<dyn Error>> {
    #[derive(serde::Deserialize)]
    #[serde(rename_all = "camelCase")]
    struct EnrollResponse {
        jwt_assertion: SecretString,
    }
    if token.trim().is_empty() {
        return Err("agent token cannot be empty".into());
    }
    let response = request(client, Method::POST, endpoint)
        .bearer_auth(token)
        .send()
        .await?;
    let response: EnrollResponse = ensure_success(response).await?.json().await?;
    if response.jwt_assertion.expose_secret().trim().is_empty() {
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

    pub fn with_metadata(
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
mod tests;
