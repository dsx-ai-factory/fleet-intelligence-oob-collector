use std::error::Error;
use std::fmt;
use std::time::Duration;

use bmc_error::sanitize_backend_error_message;
use serde::Serialize;
use serde::ser::{SerializeMap, Serializer};

use crate::models::NodeInventory;

pub const DEFAULT_BACKEND_TIMEOUT_SECONDS: u64 = 30;
pub const DEFAULT_BACKEND_CONNECT_TIMEOUT_SECONDS: u64 = 10;
const BACKEND_USER_AGENT: &str = "fleet-intelligence-oob-collector";
const AGENT_VERSION: &str = env!("CARGO_PKG_VERSION");

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
#[derive(Debug)]
pub struct BackendBatchOutcome {
    results: Vec<NodeUpsertResult>,
}

impl BackendBatchOutcome {
    /// Return the backend result for a submitted node UUID.
    pub fn node_outcome(&self, node_uuid: &str) -> BackendNodeOutcome<'_> {
        let Some(result) = self.results.iter().find(|result| {
            result
                .node_uuid
                .as_deref()
                .is_some_and(|result_uuid| result_uuid.eq_ignore_ascii_case(node_uuid))
        }) else {
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

    /// Whether the backend accepted the node with the supplied UUID.
    pub fn node_succeeded(&self, node_uuid: &str) -> bool {
        self.node_outcome(node_uuid) == BackendNodeOutcome::Succeeded
    }
}

/// Result reported by the backend for one submitted node UUID.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum BackendNodeOutcome<'a> {
    Succeeded,
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
            version: AGENT_VERSION,
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
            uuid,
            source,
            network_latency,
            primary_system_id,
            managers,
            systems,
            chassis,
            firmware,
            domain_errors,
            target_error,
        } = self.0;
        let mut map = serializer.serialize_map(Some(17))?;
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
        map.serialize_entry("uuid", uuid)?;
        map.serialize_entry("source", source)?;
        map.serialize_entry("networkLatency", network_latency)?;
        map.serialize_entry("primarySystemId", primary_system_id)?;
        map.serialize_entry("managers", managers)?;
        map.serialize_entry("systems", systems)?;
        map.serialize_entry("chassis", chassis)?;
        map.serialize_entry("firmware", firmware)?;

        let domain_errors = domain_errors
            .iter()
            .map(|error| BackendDomainError {
                domain: &error.domain,
                resource: error.resource.as_deref(),
                message: sanitize_backend_error_message(&error.message),
            })
            .collect::<Vec<_>>();
        map.serialize_entry("domainErrors", &domain_errors)?;
        map.serialize_entry(
            "targetError",
            &target_error.as_deref().map(sanitize_backend_error_message),
        )?;
        map.end()
    }
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct BackendDomainError<'a> {
    domain: &'a str,
    resource: Option<&'a str>,
    message: String,
}

#[derive(Debug, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
struct BatchUpsertResponse {
    results: Vec<NodeUpsertResult>,
}

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
        assert_eq!(value["agent"]["version"], env!("CARGO_PKG_VERSION"));
        assert_eq!(value["data"][0]["source"]["address"], "10.0.0.1");
        assert_eq!(value["data"][0]["networkLatency"]["protocol"], "icmp");
        assert_eq!(value["data"][0]["kind"], "nvswitch");
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
    }

    #[test]
    fn batch_request_redacts_urls_from_inventory_errors() {
        let mut inventory = inventory("10.0.0.1");
        inventory.domain_errors.push(crate::models::DomainError {
            domain: "systems.secureBoot".to_string(),
            resource: Some("1".to_string()),
            message: "request to https://10.0.0.1/redfish/v1/Systems/1 failed with 403".to_string(),
        });
        inventory.target_error =
            Some("could not reach HTTP://10.0.0.1/redfish/v1/Systems/1?detail=all".to_string());
        let inventories = [inventory];
        let request = build_batch_upsert_request(&inventories);

        let value = serde_json::to_value(request).expect("serialize request");
        assert_eq!(
            value["data"][0]["domainErrors"][0]["message"],
            "request to /redfish/v1/Systems/1 failed with 403"
        );
        assert_eq!(
            value["data"][0]["targetError"],
            "could not reach /redfish/v1/Systems/1"
        );

        // Sanitization is limited to the backend projection so local output
        // retains the original details for diagnostics.
        assert!(inventories[0].domain_errors[0].message.contains("https://"));
        assert!(
            inventories[0]
                .target_error
                .as_deref()
                .expect("target error")
                .contains("HTTP://")
        );
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

        assert!(outcome.node_succeeded("NODE-1"));
        assert!(!outcome.node_succeeded("node-2"));
        assert!(!outcome.node_succeeded("missing"));
        assert_eq!(
            outcome.node_outcome("NODE-2"),
            BackendNodeOutcome::Rejected {
                status: "INVALID",
                message: Some("rejected"),
            }
        );
        assert_eq!(outcome.node_outcome("missing"), BackendNodeOutcome::Missing);
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

        assert!(outcome.node_succeeded("9aee71e4-6e80-11ee-a8ac-74d4dd550187"));
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
                    .map(|inventory| NodeUpsertResult {
                        node_uuid: inventory.uuid.as_ref().map(ToString::to_string),
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
            uuid: None,
            network_latency: Default::default(),
            source: crate::models::InventorySourceInfo {
                source_type: "redfish".to_string(),
                mac: "00:11:22:33:44:55".to_string(),
                address: address.to_string(),
                host_name: None,
                redfish_version: None,
                service_uuid: None,
                vendor: None,
            },
            primary_system_id: None,
            managers: Vec::new(),
            systems: Vec::new(),
            chassis: Vec::new(),
            firmware: Vec::new(),
            domain_errors: Vec::new(),
            target_error: None,
        }
    }
}
