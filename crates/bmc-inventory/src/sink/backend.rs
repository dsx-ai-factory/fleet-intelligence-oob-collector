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

use serde::Serialize;
use serde::ser::{SerializeMap, Serializer};

use crate::COLLECTOR_VERSION;
use crate::models::NodeInventory;

pub const DEFAULT_BACKEND_TIMEOUT_SECONDS: u64 = 30;
pub const DEFAULT_BACKEND_CONNECT_TIMEOUT_SECONDS: u64 = 10;
const BACKEND_USER_AGENT: &str = "fleet-intelligence-oob-collector";

#[derive(Clone)]
pub struct BackendSink<C = HttpBackendClient> {
    client: C,
}

impl BackendSink<HttpBackendClient> {
    pub fn new(
        endpoint: impl Into<String>,
        jwt: impl Into<String>,
    ) -> Result<Self, Box<dyn Error>> {
        Ok(Self::with_client(HttpBackendClient::new(endpoint, jwt)?))
    }

    /// Update the bearer token without rebuilding the underlying HTTP client.
    pub fn set_jwt(&mut self, jwt: impl Into<String>) {
        self.client.set_jwt(jwt);
    }
}

impl<C> BackendSink<C> {
    pub fn with_client(client: C) -> Self {
        Self { client }
    }
}

impl<C> BackendSink<C>
where
    C: BackendClient,
{
    /// Write a batch and preserve the backend's result for each node.
    pub async fn write_batch(
        &self,
        inventories: &[NodeInventory],
    ) -> Result<BackendBatchOutcome, Box<dyn Error>> {
        self.client.put_inventory(inventories).await
    }
}

#[async_trait::async_trait(?Send)]
pub trait BackendClient {
    async fn put_inventory(
        &self,
        inventories: &[NodeInventory],
    ) -> Result<BackendBatchOutcome, Box<dyn Error>>;
}

#[derive(Clone)]
pub struct HttpBackendClient {
    endpoint: String,
    jwt: String,
    client: reqwest::Client,
}

impl HttpBackendClient {
    pub fn new(
        endpoint: impl Into<String>,
        jwt: impl Into<String>,
    ) -> Result<Self, Box<dyn Error>> {
        Self::with_timeout(
            endpoint,
            jwt,
            Duration::from_secs(DEFAULT_BACKEND_TIMEOUT_SECONDS),
        )
    }

    pub fn with_timeout(
        endpoint: impl Into<String>,
        jwt: impl Into<String>,
        request_timeout: Duration,
    ) -> Result<Self, Box<dyn Error>> {
        if request_timeout.is_zero() {
            return Err("backend request timeout must be greater than zero".into());
        }
        Ok(Self {
            endpoint: endpoint.into(),
            jwt: jwt.into(),
            client: backend_http_client(request_timeout)?,
        })
    }

    pub async fn enroll(endpoint: &str, agent_token: &str) -> Result<String, Box<dyn Error>> {
        Self::enroll_with_timeout(
            endpoint,
            agent_token,
            Duration::from_secs(DEFAULT_BACKEND_TIMEOUT_SECONDS),
        )
        .await
    }

    pub async fn enroll_with_timeout(
        endpoint: &str,
        agent_token: &str,
        request_timeout: Duration,
    ) -> Result<String, Box<dyn Error>> {
        if agent_token.trim().is_empty() {
            return Err("agent token cannot be empty".into());
        }
        if request_timeout.is_zero() {
            return Err("backend request timeout must be greater than zero".into());
        }

        let client = backend_http_client(request_timeout)?;
        let response = backend_request(&client, reqwest::Method::POST, endpoint)
            .bearer_auth(agent_token)
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
        let response: EnrollResponse = response.json().await?;

        if response.jwt_assertion.trim().is_empty() {
            return Err("enrollment response missing jwtAssertion field".into());
        }

        Ok(response.jwt_assertion)
    }

    fn set_jwt(&mut self, jwt: impl Into<String>) {
        self.jwt = jwt.into();
    }
}

fn backend_http_client(request_timeout: Duration) -> Result<reqwest::Client, reqwest::Error> {
    reqwest::Client::builder()
        .timeout(request_timeout)
        .connect_timeout(
            request_timeout.min(Duration::from_secs(DEFAULT_BACKEND_CONNECT_TIMEOUT_SECONDS)),
        )
        .build()
}

fn backend_request(
    client: &reqwest::Client,
    method: reqwest::Method,
    endpoint: &str,
) -> reqwest::RequestBuilder {
    client
        .request(method, endpoint)
        .header(reqwest::header::USER_AGENT, BACKEND_USER_AGENT)
}

fn response_header(response: &reqwest::Response, name: &str) -> Option<String> {
    response
        .headers()
        .get(name)
        .and_then(|value| value.to_str().ok())
        .map(str::to_string)
}

#[async_trait::async_trait(?Send)]
impl BackendClient for HttpBackendClient {
    async fn put_inventory(
        &self,
        inventories: &[NodeInventory],
    ) -> Result<BackendBatchOutcome, Box<dyn Error>> {
        let response = backend_request(&self.client, reqwest::Method::PUT, &self.endpoint)
            .bearer_auth(&self.jwt)
            .json(&build_batch_upsert_request(inventories))
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

        let response: BatchUpsertResponse = response.json().await?;
        Ok(BackendBatchOutcome {
            results: response.results,
        })
    }
}

/// Itemized response from a successful backend batch request.
#[derive(Debug, serde::Deserialize)]
pub struct BackendBatchOutcome {
    results: Vec<NodeUpsertResult>,
}

impl BackendBatchOutcome {
    /// Results correspond by position to submitted inventory observations.
    pub fn item_outcome(&self, index: usize) -> BackendNodeOutcome<'_> {
        let Some(result) = self.results.get(index) else {
            return BackendNodeOutcome::Missing;
        };
        if result.status.eq_ignore_ascii_case("OK") {
            BackendNodeOutcome::Succeeded {
                node_uuid: result.node_uuid.as_deref(),
            }
        } else {
            BackendNodeOutcome::Rejected {
                status: &result.status,
                message: result.message.as_deref(),
            }
        }
    }
}

/// Result for one submitted observation. An accepted observation may be unresolved.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum BackendNodeOutcome<'a> {
    Succeeded {
        node_uuid: Option<&'a str>,
    },
    Rejected {
        status: &'a str,
        message: Option<&'a str>,
    },
    Missing,
}

fn build_batch_upsert_request(inventories: &[NodeInventory]) -> BatchUpsertRequest<'_> {
    let data = inventories
        .iter()
        .map(|inventory| NodeUpsertRequest {
            inventory: BackendInventory(inventory),
        })
        .collect();
    BatchUpsertRequest {
        agent: AgentInfo {
            version: COLLECTOR_VERSION,
        },
        data,
    }
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

#[derive(Debug, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
struct EnrollResponse {
    jwt_assertion: String,
}

#[derive(serde::Serialize)]
#[serde(rename_all = "camelCase")]
struct BatchUpsertRequest<'a> {
    agent: AgentInfo<'static>,
    data: Vec<NodeUpsertRequest<'a>>,
}

#[derive(serde::Serialize)]
#[serde(rename_all = "camelCase")]
struct AgentInfo<'a> {
    version: &'a str,
}

#[derive(serde::Serialize)]
#[serde(rename_all = "camelCase")]
struct NodeUpsertRequest<'a> {
    #[serde(flatten)]
    inventory: BackendInventory<'a>,
}

struct BackendInventory<'a>(&'a NodeInventory);

impl Serialize for BackendInventory<'_> {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        let NodeInventory {
            schema_version,
            collected_at,
            enrolled_at,
            kind,
            name,
            compute_zone,
            node_group,
            source,
            managers,
            systems,
            chassis,
            firmware,
            fabrics,
            domain_errors,
            target_error,
            latency,
        } = self.0;
        let mut map = serializer.serialize_map(None)?;
        map.serialize_entry("schemaVersion", schema_version)?;
        map.serialize_entry("collectedAt", collected_at)?;
        map.serialize_entry("enrolledAt", enrolled_at)?;
        map.serialize_entry("kind", kind)?;
        if let Some(name) = name {
            map.serialize_entry("name", name)?;
        }
        if let Some(compute_zone) = compute_zone {
            map.serialize_entry("computeZone", compute_zone)?;
        }
        if let Some(node_group) = node_group {
            map.serialize_entry("nodeGroup", node_group)?;
        }
        map.serialize_entry("source", source)?;
        map.serialize_entry("managers", managers)?;
        map.serialize_entry("systems", systems)?;
        map.serialize_entry("chassis", chassis)?;
        map.serialize_entry("firmware", firmware)?;
        map.serialize_entry("fabrics", fabrics)?;
        map.serialize_entry("domainErrors", domain_errors)?;
        map.serialize_entry("targetError", target_error)?;
        map.serialize_entry("latency", latency)?;
        map.end()
    }
}

type BatchUpsertResponse = BackendBatchOutcome;

#[derive(Debug, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
struct NodeUpsertResult {
    #[serde(rename = "nodeUUID", alias = "nodeUuid")]
    node_uuid: Option<String>,
    status: String,
    message: Option<String>,
}

#[cfg(test)]
mod tests {
    use std::cell::RefCell;
    use std::rc::Rc;

    use super::*;

    #[test]
    fn backend_http_client_sets_user_agent() {
        let client = backend_http_client(Duration::from_secs(1)).expect("client");
        let request = backend_request(
            &client,
            reqwest::Method::POST,
            "https://backend.example.com",
        )
        .build()
        .expect("request");

        assert_eq!(
            request.headers()[reqwest::header::USER_AGENT],
            BACKEND_USER_AGENT
        );
    }

    #[tokio::test]
    async fn backend_sink_puts_node_inventory_batch() {
        let put_batches = Rc::new(RefCell::new(Vec::new()));
        let sink = BackendSink::with_client(RecordingClient {
            put_batches: Rc::clone(&put_batches),
        });

        sink.write_batch(&[inventory("10.0.0.1"), inventory("10.0.0.2")])
            .await
            .expect("write batch");

        let put_batches = put_batches.borrow();
        assert_eq!(put_batches.len(), 1);
        assert_eq!(put_batches[0].len(), 2);
        assert_eq!(put_batches[0][0], "10.0.0.1");
        assert_eq!(put_batches[0][1], "10.0.0.2");
    }

    #[test]
    fn batch_request_exports_inventory_placement_fields() {
        let mut inventory = inventory("10.0.0.1");
        inventory.kind = crate::models::TargetKind::NvSwitch;
        inventory.name = Some("node-01".to_string());
        inventory.compute_zone = Some("us-west-dc1".to_string());
        inventory.node_group = Some("gb200-training".to_string());
        let inventories = [inventory];
        let request = build_batch_upsert_request(&inventories);

        let value = serde_json::to_value(request).expect("serialize request");
        let expected_version = option_env!("FLEETINT_BUILD_VERSION")
            .filter(|version| !version.is_empty())
            .unwrap_or(env!("CARGO_PKG_VERSION"));
        assert_eq!(value["agent"]["version"], expected_version);
        assert_eq!(value["data"][0]["source"]["address"], "10.0.0.1");
        assert_eq!(value["data"][0]["kind"], "nvswitch");
        assert_eq!(value["data"][0]["source"]["mac"], "00:11:22:33:44:55");
        assert!(value["data"][0]["source"].get("hostName").is_none());
        assert!(value["data"][0].get("uuid").is_none());
        assert!(value["data"][0].get("primarySystemId").is_none());
        assert_eq!(value["data"][0]["name"], "node-01");
        assert_eq!(value["data"][0]["computeZone"], "us-west-dc1");
        assert_eq!(value["data"][0]["nodeGroup"], "gb200-training");
    }

    #[test]
    fn batch_request_omits_missing_placement_fields() {
        let inventory = inventory("10.0.0.1");
        let inventories = [inventory];
        let request = build_batch_upsert_request(&inventories);

        let value = serde_json::to_value(request).expect("serialize request");
        assert!(value["data"][0].get("computeZone").is_none());
        assert!(value["data"][0].get("nodeGroup").is_none());
        assert_eq!(value["data"][0]["latency"], serde_json::json!([]));
    }

    #[test]
    fn batch_request_exports_latency_as_top_level_item_field() {
        use crate::models::{LatencyMeasurement, LatencyMethod, LatencyStatus};

        let mut inventory = inventory("10.0.0.1");
        inventory.latency.push(LatencyMeasurement {
            method: LatencyMethod::TcpConnect,
            status: LatencyStatus::Ok,
            destination: "10.0.0.1:443".to_string(),
            measured_at: "2026-01-01T00:00:00Z".to_string(),
            attempts: 5,
            successes: 5,
            min_us: Some(412),
            median_us: Some(455),
            error: None,
        });
        let inventories = [inventory];
        let request = build_batch_upsert_request(&inventories);

        let value = serde_json::to_value(request).expect("serialize request");
        let latency = &value["data"][0]["latency"][0];
        assert_eq!(latency["method"], "tcp_connect");
        assert_eq!(latency["status"], "ok");
        assert_eq!(latency["destination"], "10.0.0.1:443");
        assert_eq!(latency["minUs"], 412);
        assert_eq!(latency["medianUs"], 455);
        assert!(latency["error"].is_null());
    }

    #[test]
    fn batch_outcome_distinguishes_success_rejection_and_missing_rows() {
        let outcome = BackendBatchOutcome {
            results: vec![
                NodeUpsertResult {
                    node_uuid: Some("node-1".to_string()),
                    status: "ok".to_string(),
                    message: None,
                },
                NodeUpsertResult {
                    node_uuid: Some("node-2".to_string()),
                    status: "INVALID".to_string(),
                    message: Some("rejected".to_string()),
                },
            ],
        };

        assert_eq!(
            outcome.item_outcome(0),
            BackendNodeOutcome::Succeeded {
                node_uuid: Some("node-1")
            }
        );
        assert_eq!(
            outcome.item_outcome(1),
            BackendNodeOutcome::Rejected {
                status: "INVALID",
                message: Some("rejected")
            }
        );
        assert_eq!(outcome.item_outcome(2), BackendNodeOutcome::Missing);
    }

    #[test]
    fn batch_response_deserializes_backend_node_uuid_spelling() {
        let response: BatchUpsertResponse = serde_json::from_value(serde_json::json!({
            "results": [{
                "message": "payload uuid missing",
                "nodeUUID": "9aee71e4-6e80-11ee-a8ac-74d4dd550187",
                "status": "OK"
            }]
        }))
        .expect("batch response");
        let outcome = BackendBatchOutcome {
            results: response.results,
        };

        assert_eq!(
            outcome.item_outcome(0),
            BackendNodeOutcome::Succeeded {
                node_uuid: Some("9aee71e4-6e80-11ee-a8ac-74d4dd550187")
            }
        );
    }

    #[test]
    fn accepted_observation_can_have_no_resolved_node() {
        let outcome: BackendBatchOutcome = serde_json::from_value(serde_json::json!({
            "results": [{"status": "OK"}]
        }))
        .expect("accepted unresolved observation");
        assert_eq!(
            outcome.item_outcome(0),
            BackendNodeOutcome::Succeeded { node_uuid: None }
        );
    }

    #[test]
    fn backend_sink_updates_jwt_without_rebuilding() {
        let mut sink =
            BackendSink::new("https://backend.example.com/v1/agent/oob_nodes", "old-jwt")
                .expect("sink");

        sink.set_jwt("new-jwt");

        assert_eq!(sink.client.jwt, "new-jwt");
    }

    #[tokio::test]
    async fn backend_sink_can_be_created_and_dropped_in_async_context() {
        let sink = BackendSink::new("https://backend.example.com/v1/agent/oob_nodes", "jwt")
            .expect("sink");

        drop(sink);
    }

    #[test]
    fn backend_client_rejects_zero_timeout() {
        let Err(err) =
            HttpBackendClient::with_timeout("https://api.example.com", "jwt", Duration::ZERO)
        else {
            panic!("zero timeout should fail");
        };

        assert!(err.to_string().contains("timeout"));
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

    #[derive(Clone)]
    struct RecordingClient {
        put_batches: Rc<RefCell<Vec<Vec<String>>>>,
    }

    #[async_trait::async_trait(?Send)]
    impl BackendClient for RecordingClient {
        async fn put_inventory(
            &self,
            inventories: &[NodeInventory],
        ) -> Result<BackendBatchOutcome, Box<dyn Error>> {
            self.put_batches.borrow_mut().push(
                inventories
                    .iter()
                    .map(|inventory| inventory.source.address.clone())
                    .collect(),
            );
            Ok(BackendBatchOutcome {
                results: inventories
                    .iter()
                    .map(|_| NodeUpsertResult {
                        node_uuid: Some("9aee71e4-6e80-11ee-a8ac-74d4dd550187".to_string()),
                        status: "OK".to_string(),
                        message: None,
                    })
                    .collect(),
            })
        }
    }

    fn inventory(address: &str) -> NodeInventory {
        NodeInventory {
            schema_version: crate::models::INVENTORY_SCHEMA_VERSION.to_string(),
            collected_at: "2026-01-01T00:00:00Z".to_string(),
            enrolled_at: None,
            kind: crate::models::TargetKind::Unknown,
            name: None,
            compute_zone: None,
            node_group: None,
            source: crate::models::InventorySourceInfo {
                source_type: "redfish".to_string(),
                mac: "00:11:22:33:44:55".to_string(),
                address: address.to_string(),
                redfish_version: None,
                service_uuid: None,
                vendor: None,
            },
            managers: Vec::new(),
            systems: Vec::new(),
            chassis: Vec::new(),
            firmware: Vec::new(),
            fabrics: Vec::new(),
            domain_errors: Vec::new(),
            target_error: None,
            latency: Vec::new(),
        }
    }
}
