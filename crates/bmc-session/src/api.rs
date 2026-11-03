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

//! The inventory API that NICo health's `http` endpoint source reads.
//!
//! - `GET /v1/targets` lists every target.
//! - `POST /v1/targets/{mac}/credentials` issues credentials for one target.
//!
//! The API listens on a Unix socket and has no authentication: file
//! permissions on the socket decide who can connect.

use std::collections::BTreeMap;
use std::future::Future;
use std::io;
use std::net::IpAddr;
use std::path::Path;
use std::sync::Arc;

use axum::extract::{Path as UrlPath, State};
use axum::http::{HeaderValue, StatusCode, header};
use axum::response::{IntoResponse, Response};
use axum::routing::{get, post};
use axum::{Json, Router};
use bmc_inventory::config::TargetKind;
use mac_address::MacAddress;
use serde::{Deserialize, Serialize};
use tokio::net::UnixListener;

use crate::{IssueError, Issued, SessionManager};

#[derive(Serialize)]
struct TargetRecord {
    mac: String,
    ip: IpAddr,
    #[serde(skip_serializing_if = "Option::is_none")]
    port: Option<u16>,
    kind: TargetKind,
    labels: BTreeMap<String, String>,
}

#[derive(Deserialize)]
#[serde(rename_all = "snake_case")]
enum Reason {
    CacheMiss,
    Rejected,
}

#[derive(Deserialize)]
struct CredentialRequest {
    reason: Reason,
    #[serde(default)]
    token: Option<String>,
}

#[derive(Serialize)]
#[serde(tag = "type", rename_all = "snake_case")]
enum CredentialResponse {
    Session { token: String },
    Basic { username: String, password: String },
}

/// The API's routes, for serving or testing.
pub fn router(manager: Arc<SessionManager>) -> Router {
    Router::new()
        .route("/v1/targets", get(targets))
        .route("/v1/targets/{mac}/credentials", post(credentials))
        .with_state(manager)
}

/// Bind the API's Unix socket, replacing a stale socket file from an earlier
/// run. The socket is created with mode `0660`, so the deployment controls
/// access through its directory and group.
///
/// # Errors
///
/// Returns an error if the socket cannot be bound.
pub fn bind(socket_path: &Path) -> io::Result<UnixListener> {
    match std::fs::remove_file(socket_path) {
        Ok(()) => {}
        Err(error) if error.kind() == io::ErrorKind::NotFound => {}
        Err(error) => return Err(error),
    }
    let listener = UnixListener::bind(socket_path)?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt as _;
        std::fs::set_permissions(socket_path, std::fs::Permissions::from_mode(0o660))?;
    }
    Ok(listener)
}

/// Serve the API on a bound socket until `shutdown` resolves.
///
/// # Errors
///
/// Returns an error if the server fails.
pub async fn serve(
    listener: UnixListener,
    manager: Arc<SessionManager>,
    shutdown: impl Future<Output = ()> + Send + 'static,
) -> io::Result<()> {
    axum::serve(listener, router(manager))
        .with_graceful_shutdown(shutdown)
        .await
}

async fn targets(State(manager): State<Arc<SessionManager>>) -> Json<Vec<TargetRecord>> {
    let mut targets: Vec<_> = manager
        .targets()
        .into_iter()
        .map(|target| TargetRecord {
            mac: target.mac.to_string(),
            ip: target.ip,
            port: target.port,
            kind: target.kind,
            labels: target.labels,
        })
        .collect();
    targets.sort_by(|left, right| left.mac.cmp(&right.mac));
    Json(targets)
}

async fn credentials(
    State(manager): State<Arc<SessionManager>>,
    UrlPath(mac): UrlPath<String>,
    Json(request): Json<CredentialRequest>,
) -> Response {
    let Ok(mac) = mac.parse::<MacAddress>() else {
        return error(StatusCode::BAD_REQUEST, "invalid MAC address");
    };
    let rejected_token = match request.reason {
        Reason::CacheMiss => None,
        Reason::Rejected => request.token.as_deref(),
    };
    match manager.credentials(mac, rejected_token).await {
        Ok(issued) => {
            let body = match issued {
                Issued::Session { token } => CredentialResponse::Session { token },
                Issued::Basic { username, password } => {
                    CredentialResponse::Basic { username, password }
                }
            };
            let mut response = Json(body).into_response();
            response
                .headers_mut()
                .insert(header::CACHE_CONTROL, HeaderValue::from_static("no-store"));
            response
        }
        Err(IssueError::UnknownTarget) => error(StatusCode::NOT_FOUND, "unknown target"),
        Err(IssueError::CredentialsRejected) => error(
            StatusCode::CONFLICT,
            "BMC rejected the configured credentials; retry after the next collection cycle",
        ),
        Err(failure @ IssueError::Unavailable(_)) => {
            tracing::warn!(bmc_mac_address = %mac, error = %failure, "failed to issue BMC credentials");
            error(StatusCode::BAD_GATEWAY, "BMC login failed")
        }
    }
}

fn error(status: StatusCode, message: &'static str) -> Response {
    (status, Json(serde_json::json!({ "error": message }))).into_response()
}

#[cfg(test)]
mod tests {
    use serde_json::{Value, json};

    use super::*;
    use crate::tests::{FakeBmc, manager, start_bmc, target};

    #[tokio::test]
    async fn serves_targets_and_credentials_on_a_unix_socket() {
        let address = start_bmc(FakeBmc::default()).await;
        let manager = Arc::new(manager(target(&address, "good")));
        let dir = std::env::temp_dir().join(format!("bmc-session-api-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let socket = dir.join("api.sock");
        let (stop, stopped) = tokio::sync::oneshot::channel::<()>();
        let server = tokio::spawn({
            let socket = socket.clone();
            async move {
                serve(bind(&socket).unwrap(), manager, async {
                    let _ = stopped.await;
                })
                .await
            }
        });
        while !socket.exists() {
            tokio::task::yield_now().await;
        }
        let client = reqwest::Client::builder()
            .unix_socket(socket.as_path())
            .build()
            .unwrap();

        let targets: Value = client
            .get("http://localhost/v1/targets")
            .send()
            .await
            .unwrap()
            .json()
            .await
            .unwrap();
        assert_eq!(
            targets,
            json!([{"mac": "00:11:22:33:44:55", "ip": "127.0.0.1", "kind": "compute", "labels": {"rack": "r1"}}])
        );

        let response = client
            .post("http://localhost/v1/targets/00:11:22:33:44:55/credentials")
            .json(&json!({"reason": "cache_miss"}))
            .send()
            .await
            .unwrap();
        assert_eq!(response.headers()["cache-control"], "no-store");
        assert_eq!(
            response.json::<Value>().await.unwrap(),
            json!({"type": "session", "token": "token-1"})
        );

        let unknown = client
            .post("http://localhost/v1/targets/aa:aa:aa:aa:aa:aa/credentials")
            .json(&json!({"reason": "cache_miss"}))
            .send()
            .await
            .unwrap();
        assert_eq!(unknown.status(), StatusCode::NOT_FOUND);

        stop.send(()).unwrap();
        server.await.unwrap().unwrap();
        std::fs::remove_dir_all(&dir).unwrap();
    }
}
