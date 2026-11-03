// Copyright (c) 2026, NVIDIA CORPORATION & AFFILIATES. All rights reserved.
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

//! Redfish session tokens for BMC consumers that run beside the collector.
//!
//! [`SessionManager`] logs in to each BMC once with the target's configured
//! credentials and hands out the session token. [`api`] serves it to NICo
//! health over a Unix socket.

pub mod api;

use std::collections::HashMap;
use std::sync::{Arc, RwLock};
use std::time::Duration;

use bmc_dispatch::{DispatchError, Dispatcher, FailureKind};
use bmc_inventory::config::Target;
use http::HeaderMap;
use mac_address::MacAddress;
use nv_redfish::Bmc as _;
use nv_redfish::bmc_http::reqwest::{BmcError, Client, ClientParams};
use nv_redfish::bmc_http::{BmcCredentials, CacheSettings, HttpBmc, HttpClient as _};
use nv_redfish::core::ODataId;
use tokio::sync::Mutex;
use url::Url;

const SESSIONS_PATH: &str = "/redfish/v1/SessionService/Sessions";
const USER_AGENT: &str = "fleet-intelligence-oob-collector";

/// Credentials issued to a consumer for one BMC.
#[derive(Clone, PartialEq, Eq)]
pub enum Issued {
    /// A Redfish session token for `X-Auth-Token`.
    Session { token: String },
    /// Basic credentials, for a BMC without a usable SessionService.
    Basic { username: String, password: String },
}

impl std::fmt::Debug for Issued {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Session { .. } => formatter.write_str("Session([REDACTED])"),
            Self::Basic { username, .. } => formatter
                .debug_struct("Basic")
                .field("username", username)
                .finish_non_exhaustive(),
        }
    }
}

/// Why credentials could not be issued.
#[derive(Debug)]
pub enum IssueError {
    /// No configured target has this MAC.
    UnknownTarget,
    /// The BMC rejected the configured credentials this cycle. The collector
    /// does not log in again until its next cycle, to avoid lockout.
    CredentialsRejected,
    /// The login failed for another reason, such as an unreachable BMC.
    Unavailable(String),
}

impl std::fmt::Display for IssueError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::UnknownTarget => formatter.write_str("unknown target"),
            Self::CredentialsRejected => {
                formatter.write_str("BMC rejected the configured credentials")
            }
            Self::Unavailable(reason) => write!(formatter, "BMC login failed: {reason}"),
        }
    }
}

impl std::error::Error for IssueError {}

/// One consumer's authentication state for one BMC.
enum State {
    Session {
        token: String,
        location: ODataId,
        /// The configured credentials the session was created with.
        login: (String, String),
    },
    /// The BMC has no usable SessionService.
    Basic,
}

/// Issues and maintains one Redfish session per BMC for NICo health.
pub struct SessionManager {
    client: Client,
    dispatcher: Arc<Dispatcher>,
    targets: RwLock<HashMap<MacAddress, Target>>,
    /// Per-BMC state, each behind its own lock so one slow login never
    /// blocks another BMC.
    states: Mutex<HashMap<MacAddress, Arc<Mutex<Option<State>>>>>,
}

impl SessionManager {
    /// Build a manager whose logins share the dispatcher's per-BMC limits.
    ///
    /// # Errors
    ///
    /// Returns an error if the HTTP client cannot be built.
    pub fn new(
        insecure_tls: bool,
        request_timeout: Duration,
        dispatcher: Arc<Dispatcher>,
    ) -> Result<Self, BmcError> {
        let client = Client::with_params(
            ClientParams::new()
                .timeout(request_timeout)
                .connect_timeout(request_timeout.min(Duration::from_secs(10)))
                .accept_invalid_certs(insecure_tls)
                .user_agent(USER_AGENT),
        )?;
        Ok(Self {
            client,
            dispatcher,
            targets: RwLock::new(HashMap::new()),
            states: Mutex::new(HashMap::new()),
        })
    }

    /// Replace the target snapshot. Call once per collection cycle, after the
    /// dispatcher's endpoints are synchronized.
    pub fn update_targets(&self, targets: &[Target]) {
        let snapshot = targets
            .iter()
            .map(|target| (target.mac, target.clone()))
            .collect();
        if let Ok(mut current) = self.targets.write() {
            *current = snapshot;
        }
    }

    /// The current target snapshot.
    #[must_use]
    pub fn targets(&self) -> Vec<Target> {
        self.targets
            .read()
            .map(|targets| targets.values().cloned().collect())
            .unwrap_or_default()
    }

    /// Credentials for `mac`. `rejected_token` is a token the consumer's BMC
    /// request was rejected with; the session is replaced only if that token
    /// is still current, so concurrent rejections cause one login.
    ///
    /// # Errors
    ///
    /// See [`IssueError`].
    pub async fn credentials(
        &self,
        mac: MacAddress,
        rejected_token: Option<&str>,
    ) -> Result<Issued, IssueError> {
        let target = self
            .targets
            .read()
            .ok()
            .and_then(|targets| targets.get(&mac).cloned())
            .ok_or(IssueError::UnknownTarget)?;
        let login = (target.username.clone(), target.password.clone());
        let slot = Arc::clone(self.states.lock().await.entry(mac).or_default());
        let mut state = slot.lock().await;

        match state.as_ref() {
            Some(State::Basic) => {
                return Ok(Issued::Basic {
                    username: login.0,
                    password: login.1,
                });
            }
            Some(State::Session {
                token,
                login: current,
                ..
            }) if *current == login && rejected_token.is_none_or(|rejected| rejected != token) => {
                return Ok(Issued::Session {
                    token: token.clone(),
                });
            }
            _ => {}
        }

        if let Some(State::Session {
            token, location, ..
        }) = state.take()
        {
            self.logout(&target, token, location).await;
        }
        let issued = self.login(&target).await;
        *state = match &issued {
            Ok((token, location)) => Some(State::Session {
                token: token.clone(),
                location: location.clone(),
                login: login.clone(),
            }),
            Err(LoginError::NoSessionService) => Some(State::Basic),
            Err(_) => None,
        };
        match issued {
            Ok((token, _)) => Ok(Issued::Session { token }),
            Err(LoginError::NoSessionService) => Ok(Issued::Basic {
                username: login.0,
                password: login.1,
            }),
            Err(LoginError::Rejected) => Err(IssueError::CredentialsRejected),
            Err(LoginError::Failed(reason)) => Err(IssueError::Unavailable(reason)),
        }
    }

    /// Delete every session this manager created. Call on shutdown.
    pub async fn logout_all(&self) {
        let states: Vec<_> = self.states.lock().await.drain().collect();
        for (mac, slot) in states {
            let Some(target) = self
                .targets
                .read()
                .ok()
                .and_then(|targets| targets.get(&mac).cloned())
            else {
                continue;
            };
            if let Some(State::Session {
                token, location, ..
            }) = slot.lock().await.take()
            {
                self.logout(&target, token, location).await;
            }
        }
    }

    async fn login(&self, target: &Target) -> Result<(String, ODataId), LoginError> {
        let url = bmc_url(&target.address).map_err(LoginError::Failed)?;
        let gate = self
            .dispatcher
            .inventory_gate(&target.mac.to_string())
            .ok_or_else(|| LoginError::Failed("no dispatcher endpoint for target".into()))?;
        let credentials = BmcCredentials::new(target.username.clone(), target.password.clone());
        let bmc = HttpBmc::new(
            self.client.clone(),
            url,
            BmcCredentials::anonymous(),
            CacheSettings::with_capacity(1),
        );
        let body = serde_json::json!({
            "UserName": target.username,
            "Password": target.password,
        });
        let sessions = ODataId::from(SESSIONS_PATH.to_string());
        // The credential latch makes a rejected login fail locally for the rest
        // of the cycle, so a wrong password costs the BMC one failed attempt.
        let response = gate
            .execute_classified(
                &credentials,
                bmc.create_session::<_, serde_json::Value>(&sessions, &body),
                classify,
            )
            .await;
        match response {
            Ok(response) => Ok((response.auth_token, response.location)),
            Err(DispatchError::CredentialsRejected) => Err(LoginError::Rejected),
            Err(DispatchError::Transport(error)) => Err(match status(&error) {
                Some(401) => LoginError::Rejected,
                Some(404 | 405 | 501) => LoginError::NoSessionService,
                _ => LoginError::Failed(error_summary(&error)),
            }),
            Err(error) => Err(LoginError::Failed(error.to_string())),
        }
    }

    /// Best-effort session deletion; an unreachable BMC expires it instead.
    async fn logout(&self, target: &Target, token: String, location: ODataId) {
        let Ok(base) = bmc_url(&target.address) else {
            return;
        };
        let Ok(url) = base.join(&location.to_string()) else {
            return;
        };
        let Some(gate) = self.dispatcher.inventory_gate(&target.mac.to_string()) else {
            return;
        };
        let credentials = BmcCredentials::token(token);
        let result = gate
            .execute(
                self.client
                    .delete::<serde_json::Value>(url, &credentials, &HeaderMap::new()),
            )
            .await;
        if let Err(error) = result {
            tracing::debug!(bmc_mac_address = %target.mac, %error, "failed to delete BMC session");
        }
    }
}

enum LoginError {
    Rejected,
    NoSessionService,
    Failed(String),
}

fn status(error: &BmcError) -> Option<u16> {
    match error {
        BmcError::InvalidResponse { status, .. } => Some(status.as_u16()),
        _ => None,
    }
}

fn classify(error: &BmcError) -> FailureKind {
    match status(error) {
        Some(401) => FailureKind::CredentialsRejected,
        _ => FailureKind::Permanent,
    }
}

/// A short error description without the response body, which may echo
/// credentials.
fn error_summary(error: &BmcError) -> String {
    match error {
        BmcError::InvalidResponse { status, .. } => format!("HTTP {status}"),
        BmcError::ReqwestError(error) if error.is_timeout() => "request timed out".into(),
        BmcError::ReqwestError(error) if error.is_connect() => "connection failed".into(),
        BmcError::ReqwestError(_) => "request failed".into(),
        _ => "invalid session response".into(),
    }
}

fn bmc_url(address: &str) -> Result<Url, String> {
    let address = if address.starts_with("http://") || address.starts_with("https://") {
        address.to_string()
    } else {
        format!("https://{address}")
    };
    address
        .parse()
        .map_err(|error| format!("invalid BMC address: {error}"))
}

#[cfg(test)]
mod tests {
    use std::sync::Mutex as StdMutex;
    use std::sync::atomic::{AtomicUsize, Ordering};

    use axum::Router;
    use axum::extract::State;
    use axum::http::{HeaderMap as AxumHeaders, StatusCode};
    use axum::response::IntoResponse;
    use axum::routing::{delete, post};
    use bmc_dispatch::DispatcherConfig;
    use bmc_inventory::config::TargetKind;

    use super::*;

    const MAC: &str = "00:11:22:33:44:55";

    /// A BMC whose SessionService accepts one password.
    #[derive(Clone, Default)]
    pub(crate) struct FakeBmc {
        pub(crate) logins: Arc<AtomicUsize>,
        pub(crate) deleted: Arc<StdMutex<Vec<String>>>,
        pub(crate) no_session_service: bool,
    }

    pub(crate) async fn start_bmc(bmc: FakeBmc) -> String {
        async fn login(
            State(bmc): State<FakeBmc>,
            axum::Json(body): axum::Json<serde_json::Value>,
        ) -> axum::response::Response {
            if bmc.no_session_service {
                return StatusCode::NOT_FOUND.into_response();
            }
            if body["Password"] != "good" {
                return StatusCode::UNAUTHORIZED.into_response();
            }
            let id = bmc.logins.fetch_add(1, Ordering::SeqCst) + 1;
            let mut headers = AxumHeaders::new();
            headers.insert("X-Auth-Token", format!("token-{id}").parse().unwrap());
            headers.insert(
                "Location",
                format!("/redfish/v1/SessionService/Sessions/{id}")
                    .parse()
                    .unwrap(),
            );
            (
                StatusCode::CREATED,
                headers,
                axum::Json(serde_json::json!({
                    "@odata.id": format!("/redfish/v1/SessionService/Sessions/{id}"),
                    "Id": id.to_string(),
                })),
            )
                .into_response()
        }
        async fn logout(
            State(bmc): State<FakeBmc>,
            axum::extract::Path(id): axum::extract::Path<String>,
        ) -> StatusCode {
            bmc.deleted.lock().unwrap().push(id);
            StatusCode::NO_CONTENT
        }
        let app = Router::new()
            .route(SESSIONS_PATH, post(login))
            .route(&format!("{SESSIONS_PATH}/{{id}}"), delete(logout))
            .with_state(bmc);
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();
        tokio::spawn(async move { axum::serve(listener, app).await });
        format!("http://{address}")
    }

    pub(crate) fn target(address: &str, password: &str) -> Target {
        Target {
            kind: TargetKind::Compute,
            name: None,
            compute_zone: None,
            node_group: None,
            ip: "127.0.0.1".parse().unwrap(),
            port: None,
            mac: MAC.parse().unwrap(),
            address: address.to_string(),
            credentials_profile: None,
            username: "admin".into(),
            password: password.into(),
            labels: [("rack".to_string(), "r1".to_string())].into(),
        }
    }

    pub(crate) fn manager(target: Target) -> SessionManager {
        let dispatcher = Dispatcher::start(
            DispatcherConfig {
                global_max_in_flight: 4,
                bmc_max_in_flight: 1,
                bmc_requests_per_second: 100,
                bmc_burst: 100,
                workflow_queue_capacity: 8,
            },
            [MAC.to_string()],
        )
        .unwrap();
        let manager =
            SessionManager::new(false, Duration::from_secs(5), Arc::new(dispatcher)).unwrap();
        manager.update_targets(&[target]);
        manager
    }

    fn session(token: &str) -> Issued {
        Issued::Session {
            token: token.into(),
        }
    }

    #[tokio::test]
    async fn reuses_the_session_until_its_current_token_is_rejected() {
        let bmc = FakeBmc::default();
        let address = start_bmc(bmc.clone()).await;
        let manager = manager(target(&address, "good"));
        let mac = MAC.parse().unwrap();

        assert_eq!(
            manager.credentials(mac, None).await.unwrap(),
            session("token-1")
        );
        assert_eq!(
            manager.credentials(mac, None).await.unwrap(),
            session("token-1")
        );
        // A stale rejection does not replace the newer session.
        assert_eq!(
            manager.credentials(mac, Some("token-0")).await.unwrap(),
            session("token-1")
        );
        assert_eq!(
            manager.credentials(mac, Some("token-1")).await.unwrap(),
            session("token-2")
        );
        assert_eq!(bmc.logins.load(Ordering::SeqCst), 2);
        assert_eq!(*bmc.deleted.lock().unwrap(), ["1"]);

        manager.logout_all().await;
        assert_eq!(*bmc.deleted.lock().unwrap(), ["1", "2"]);
    }

    #[tokio::test]
    async fn rejected_password_is_not_retried_until_the_next_cycle() {
        let bmc = FakeBmc::default();
        let address = start_bmc(bmc.clone()).await;
        let manager = manager(target(&address, "bad"));
        let mac = MAC.parse().unwrap();

        for _ in 0..2 {
            assert!(matches!(
                manager.credentials(mac, None).await,
                Err(IssueError::CredentialsRejected)
            ));
        }
        assert_eq!(bmc.logins.load(Ordering::SeqCst), 0);

        manager.dispatcher.clear_credential_rejections();
        manager.update_targets(&[target(&address, "good")]);
        assert_eq!(
            manager.credentials(mac, None).await.unwrap(),
            session("token-1")
        );
    }

    #[tokio::test]
    async fn falls_back_to_basic_without_a_session_service() {
        let address = start_bmc(FakeBmc {
            no_session_service: true,
            ..FakeBmc::default()
        })
        .await;
        let manager = manager(target(&address, "good"));

        assert_eq!(
            manager
                .credentials(MAC.parse().unwrap(), None)
                .await
                .unwrap(),
            Issued::Basic {
                username: "admin".into(),
                password: "good".into(),
            }
        );
    }

    #[tokio::test]
    async fn unknown_targets_are_reported() {
        let manager = manager(target("http://127.0.0.1:9", "good"));
        assert!(matches!(
            manager
                .credentials("aa:aa:aa:aa:aa:aa".parse().unwrap(), None)
                .await,
            Err(IssueError::UnknownTarget)
        ));
    }
}
