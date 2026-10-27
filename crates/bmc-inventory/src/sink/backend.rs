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
use std::time::Duration;

pub use backend_client::{BackendHttpError, DEFAULT_BACKEND_TIMEOUT_SECONDS};
use backend_client::{BackendUrl, ensure_success, http_client_builder, request as backend_request};
use serde::Serialize;
use serde::ser::{SerializeMap, Serializer};
use url::Url;

use crate::COLLECTOR_VERSION;
use crate::models::NodeInventory;

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CollectorConfig {
    pub runtime: RuntimeConfig,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub nico_health: Option<NicoHealthConfig>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub otel_gateway: Option<OtelGatewayConfig>,
}

/// Configured NICo behavior; absent fields are unknown outside Helm.
#[derive(Clone, Debug, Default, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct NicoHealthConfig {
    pub enabled: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub log_level: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub telemetry_destination: Option<String>,
}

/// Configured gateway behavior, independent of readiness.
#[derive(Clone, Debug, Default, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct OtelGatewayConfig {
    pub enabled: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub log_level: Option<String>,
}

/// Non-sensitive runtime settings shared by every node in an inventory batch.
#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RuntimeConfig {
    pub inventory_interval_seconds: u64,
    pub target_concurrency: usize,
    pub bmc_max_in_flight: u32,
    pub bmc_requests_per_second: u64,
    pub bmc_burst: u64,
    pub timeout_seconds: u64,
    pub insecure_tls: bool,
    pub latency_probe: bool,
    pub log_level: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub proxy_enabled: Option<bool>,
}

#[derive(Clone)]
pub struct BackendSink<C = HttpBackendClient> {
    client: C,
}

impl BackendSink<HttpBackendClient> {
    pub fn new(
        endpoint: impl Into<String>,
        jwt: impl Into<String>,
        config: CollectorConfig,
    ) -> Result<Self, Box<dyn Error>> {
        Ok(Self::with_client(HttpBackendClient::new(
            endpoint, jwt, config,
        )?))
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
    endpoint: Url,
    jwt: String,
    client: reqwest::Client,
    collector_config: CollectorConfig,
}

impl HttpBackendClient {
    pub fn new(
        endpoint: impl Into<String>,
        jwt: impl Into<String>,
        config: CollectorConfig,
    ) -> Result<Self, Box<dyn Error>> {
        Self::with_timeout(
            endpoint,
            jwt,
            Duration::from_secs(DEFAULT_BACKEND_TIMEOUT_SECONDS),
            config,
        )
    }

    pub fn with_timeout(
        endpoint: impl Into<String>,
        jwt: impl Into<String>,
        request_timeout: Duration,
        config: CollectorConfig,
    ) -> Result<Self, Box<dyn Error>> {
        if request_timeout.is_zero() {
            return Err("backend request timeout must be greater than zero".into());
        }
        Ok(Self {
            endpoint: BackendUrl::parse(&endpoint.into())?.as_url().clone(),
            jwt: jwt.into(),
            client: http_client_builder(request_timeout, reqwest::redirect::Policy::default())?
                .build()?,
            collector_config: config,
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
        let endpoint = BackendUrl::parse(endpoint)?;
        let client =
            http_client_builder(request_timeout, reqwest::redirect::Policy::default())?.build()?;
        backend_client::enroll(&client, endpoint.as_url(), agent_token).await
    }

    fn set_jwt(&mut self, jwt: impl Into<String>) {
        self.jwt = jwt.into();
    }
}

#[async_trait::async_trait(?Send)]
impl BackendClient for HttpBackendClient {
    async fn put_inventory(
        &self,
        inventories: &[NodeInventory],
    ) -> Result<BackendBatchOutcome, Box<dyn Error>> {
        let response = backend_request(&self.client, reqwest::Method::PUT, &self.endpoint)
            .bearer_auth(self.jwt.as_str())
            .json(&build_batch_upsert_request(
                inventories,
                &self.collector_config,
            ))
            .send()
            .await?;
        let response = ensure_success(response).await?;

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
            BackendNodeOutcome::Succeeded
        } else {
            BackendNodeOutcome::Rejected {
                status: &result.status,
                message: result.message.as_deref(),
            }
        }
    }
}

/// Acceptance result for one submitted observation, matched by input position.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum BackendNodeOutcome<'a> {
    Succeeded,
    Rejected {
        status: &'a str,
        message: Option<&'a str>,
    },
    Missing,
}

fn build_batch_upsert_request<'a>(
    inventories: &'a [NodeInventory],
    config: &'a CollectorConfig,
) -> BatchUpsertRequest<'a> {
    let data = inventories
        .iter()
        .map(|inventory| NodeUpsertRequest {
            inventory: BackendInventory(inventory),
        })
        .collect();
    BatchUpsertRequest {
        collector: CollectorInfo {
            version: COLLECTOR_VERSION,
            config,
        },
        data,
    }
}

#[derive(serde::Serialize)]
#[serde(rename_all = "camelCase")]
struct BatchUpsertRequest<'a> {
    collector: CollectorInfo<'a>,
    data: Vec<NodeUpsertRequest<'a>>,
}

#[derive(serde::Serialize)]
#[serde(rename_all = "camelCase")]
struct CollectorInfo<'a> {
    version: &'a str,
    config: &'a CollectorConfig,
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
        let client =
            http_client_builder(Duration::from_secs(1), reqwest::redirect::Policy::default())
                .unwrap()
                .build()
                .expect("client");
        let request = backend_request(
            &client,
            reqwest::Method::POST,
            &"https://backend.example.com".parse().unwrap(),
        )
        .build()
        .expect("request");

        assert_eq!(
            request.headers()[reqwest::header::USER_AGENT],
            backend_client::BACKEND_USER_AGENT
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
        let config = collector_config();
        let request = build_batch_upsert_request(&inventories, &config);

        let value = serde_json::to_value(request).expect("serialize request");
        let expected_version = option_env!("FLEETINT_BUILD_VERSION")
            .filter(|version| !version.is_empty())
            .unwrap_or(env!("CARGO_PKG_VERSION"));
        assert_eq!(value["collector"]["version"], expected_version);
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
        let config = collector_config();
        let request = build_batch_upsert_request(&inventories, &config);

        let value = serde_json::to_value(request).expect("serialize request");
        assert!(value["data"][0].get("computeZone").is_none());
        assert!(value["data"][0].get("nodeGroup").is_none());
        assert_eq!(value["data"][0]["latency"], serde_json::json!([]));
        assert!(value.get("agent").is_none());
        assert_eq!(value["collector"]["config"]["runtime"]["logLevel"], "info");
    }

    #[test]
    fn batch_reports_config_once_and_preserves_optional_enablement() {
        let config = collector_config();
        let sink = BackendSink::new(
            "https://api.example.com",
            "must-not-be-reported",
            config.clone(),
        )
        .unwrap();
        let inventories = [inventory("10.0.0.1"), inventory("10.0.0.2")];
        let request = build_batch_upsert_request(&inventories, &sink.client.collector_config);
        let value = serde_json::to_value(request).unwrap();
        assert_eq!(
            value["collector"]["config"],
            serde_json::json!({
                "runtime": {
                    "inventoryIntervalSeconds": 3600,
                    "targetConcurrency": 8,
                    "bmcMaxInFlight": 2,
                    "bmcRequestsPerSecond": 5,
                    "bmcBurst": 6,
                    "timeoutSeconds": 15,
                    "insecureTls": false,
                    "latencyProbe": false,
                    "logLevel": "info"
                }
            })
        );
        for item in value["data"].as_array().unwrap() {
            assert!(item.get("collector").is_none());
        }
        assert!(!value.to_string().contains("must-not-be-reported"));

        for (nico, gateway) in [
            (Some(false), Some(false)),
            (Some(true), Some(true)),
            (None, Some(false)),
            (Some(false), None),
        ] {
            let config = CollectorConfig {
                nico_health: nico.map(|enabled| NicoHealthConfig {
                    enabled,
                    ..Default::default()
                }),
                otel_gateway: gateway.map(|enabled| OtelGatewayConfig {
                    enabled,
                    ..Default::default()
                }),
                ..config.clone()
            };
            let value =
                serde_json::to_value(build_batch_upsert_request(&inventories, &config)).unwrap();
            assert_eq!(
                value["collector"]["config"].get("nicoHealth"),
                nico.map(|enabled| serde_json::json!({"enabled": enabled}))
                    .as_ref()
            );
            assert_eq!(
                value["collector"]["config"].get("otelGateway"),
                gateway
                    .map(|enabled| serde_json::json!({"enabled": enabled}))
                    .as_ref()
            );
        }
    }

    #[test]
    fn batch_reports_behavior_settings_without_exporting_deployment_details() {
        let mut config = collector_config();
        config.runtime.proxy_enabled = Some(false);
        config.nico_health = Some(NicoHealthConfig {
            enabled: true,
            log_level: Some("debug".into()),
            telemetry_destination: Some("external".into()),
        });
        config.otel_gateway = Some(OtelGatewayConfig {
            enabled: false,
            log_level: Some("warn".into()),
        });
        let inventories = [inventory("10.0.0.1")];
        let value =
            serde_json::to_value(build_batch_upsert_request(&inventories, &config)).unwrap();
        let reported = &value["collector"]["config"];
        assert_eq!(reported["runtime"]["proxyEnabled"], false);
        assert_eq!(
            reported["nicoHealth"],
            serde_json::json!({
                "enabled": true, "logLevel": "debug", "telemetryDestination": "external"
            })
        );
        assert_eq!(
            reported["otelGateway"],
            serde_json::json!({"enabled": false, "logLevel": "warn"})
        );
        assert_eq!(reported.as_object().unwrap().len(), 3);
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
        let config = collector_config();
        let request = build_batch_upsert_request(&inventories, &config);

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
                    status: "ok".to_string(),
                    message: None,
                },
                NodeUpsertResult {
                    status: "INVALID".to_string(),
                    message: Some("rejected".to_string()),
                },
            ],
        };

        assert_eq!(outcome.item_outcome(0), BackendNodeOutcome::Succeeded);
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
    fn inventory_acknowledgement_accepts_placement_without_uuid() {
        let outcome: BackendBatchOutcome = serde_json::from_value(serde_json::json!({
            "results": [{"status": "OK", "computeZone": "zone-1", "nodeGroup": "group-1"}]
        }))
        .expect("current backend acknowledgement");
        assert_eq!(outcome.item_outcome(0), BackendNodeOutcome::Succeeded);
    }

    #[test]
    fn backend_sink_updates_jwt_without_rebuilding() {
        let mut sink = BackendSink::new(
            "https://backend.example.com/v1/agent/oob_nodes",
            "old-jwt",
            collector_config(),
        )
        .expect("sink");

        sink.set_jwt("new-jwt");

        assert_eq!(sink.client.jwt.as_str(), "new-jwt");
    }

    #[tokio::test]
    async fn backend_sink_can_be_created_and_dropped_in_async_context() {
        let sink = BackendSink::new(
            "https://backend.example.com/v1/agent/oob_nodes",
            "jwt",
            collector_config(),
        )
        .expect("sink");

        drop(sink);
    }

    #[test]
    fn backend_client_rejects_zero_timeout() {
        let Err(err) = HttpBackendClient::with_timeout(
            "https://api.example.com",
            "jwt",
            Duration::ZERO,
            collector_config(),
        ) else {
            panic!("zero timeout should fail");
        };

        assert!(err.to_string().contains("timeout"));
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
                        status: "OK".to_string(),
                        message: None,
                    })
                    .collect(),
            })
        }
    }

    fn collector_config() -> CollectorConfig {
        CollectorConfig {
            runtime: RuntimeConfig {
                inventory_interval_seconds: 3600,
                target_concurrency: 8,
                bmc_max_in_flight: 2,
                bmc_requests_per_second: 5,
                bmc_burst: 6,
                timeout_seconds: 15,
                insecure_tls: false,
                latency_probe: false,
                log_level: "info".to_string(),
                proxy_enabled: None,
            },
            nico_health: None,
            otel_gateway: None,
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
