//! Shared backend session/helper for the OOB agent runtime.
//!
//! Centralizes the agent's backend concerns that inventory and attestation both
//! depend on: API endpoint/URL resolution, the JWT/enrollment lifecycle, and
//! auth-failure detection. The typed request/response handling stays inside the
//! `bmc-inventory` and `bmc-attestation` crate sinks; this module only owns the
//! cross-cutting session state.

use std::error::Error;

use bmc_attestation::config as attestation_config;
use bmc_attestation::sink::BackendHttpError as AttestationHttpError;
use bmc_attestation::source::now_rfc3339;
use bmc_inventory::sink::{
    BackendHttpError as InventoryHttpError, HttpBackendClient as InventoryHttpBackendClient,
};

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
            return Err("backend URL must be the API base URL without a /v1/agent path suffix".into());
        }

        Ok(Self {
            enroll_endpoint: format!("{base_url}/v1/agent/enroll"),
            oob_nodes_endpoint: format!("{base_url}/v1/agent/oob_nodes"),
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
}

impl BackendSession {
    /// Perform the initial enrollment and capture the issued JWT + timestamp.
    pub async fn enroll(
        enroll_endpoint: String,
        agent_token: String,
    ) -> Result<Self, Box<dyn Error>> {
        let Enrollment { jwt, enrolled_at } = enroll_once(&enroll_endpoint, &agent_token).await?;
        Ok(Self {
            agent_token,
            enroll_endpoint,
            jwt,
            enrolled_at,
        })
    }

    pub fn jwt(&self) -> &str {
        &self.jwt
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
        let Enrollment { jwt, enrolled_at } =
            enroll_once(&self.enroll_endpoint, &self.agent_token).await?;
        self.jwt = jwt;
        self.enrolled_at = enrolled_at;
        Ok(())
    }
}

struct Enrollment {
    jwt: String,
    enrolled_at: String,
}

async fn enroll_once(enroll_endpoint: &str, agent_token: &str) -> Result<Enrollment, Box<dyn Error>> {
    let enroll_endpoint = enroll_endpoint.to_string();
    let agent_token = agent_token.to_string();
    let jwt = tokio::task::spawn_blocking(move || {
        InventoryHttpBackendClient::enroll(&enroll_endpoint, &agent_token)
            .map_err(|err| err.to_string())
    })
    .await?
    .map_err(|err| -> Box<dyn Error> { err.into() })?;
    Ok(Enrollment {
        jwt,
        enrolled_at: now_rfc3339(),
    })
}

/// Whether a backend error represents a JWT rejection (HTTP 401/403) from either
/// the inventory or attestation upload path, i.e. a signal to re-enroll.
pub fn is_auth_failure(err: &(dyn Error + 'static)) -> bool {
    if let Some(err) = err.downcast_ref::<InventoryHttpError>() {
        return matches!(err.status(), 401 | 403);
    }
    if let Some(err) = err.downcast_ref::<AttestationHttpError>() {
        return matches!(err.status(), 401 | 403);
    }
    false
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
        assert!(required_backend_option(None, "backend URL").is_err());
        assert!(required_backend_option(Some("   ".to_string()), "agent token").is_err());
        assert_eq!(
            required_backend_option(Some("value".to_string()), "backend URL").expect("value"),
            "value"
        );
    }

    #[test]
    fn is_auth_failure_detects_jwt_rejection_statuses() {
        let unauthorized: Box<dyn Error> = Box::new(InventoryHttpError::new(401, "expired token"));
        let forbidden: Box<dyn Error> =
            Box::new(AttestationHttpError::new(403, "invalid token"));
        let server_error: Box<dyn Error> =
            Box::new(InventoryHttpError::new(500, "backend failed"));

        assert!(is_auth_failure(unauthorized.as_ref()));
        assert!(is_auth_failure(forbidden.as_ref()));
        assert!(!is_auth_failure(server_error.as_ref()));
    }
}
