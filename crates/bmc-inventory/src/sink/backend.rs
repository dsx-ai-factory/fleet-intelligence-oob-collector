use std::collections::BTreeMap;
use std::error::Error;
use std::fmt;
use std::sync::Arc;
use std::time::Duration;

use bmc_error::sanitize_backend_error_message;
use serde::Serialize;
use serde::ser::{SerializeMap, Serializer};

use crate::models::NodeInventory;

use super::InventorySink;

pub const DEFAULT_BACKEND_TIMEOUT_SECONDS: u64 = 30;
pub const DEFAULT_BACKEND_CONNECT_TIMEOUT_SECONDS: u64 = 10;
const BACKEND_USER_AGENT: &str = "fleet-intelligence-oob-collector";
const AGENT_VERSION: &str = env!("CARGO_PKG_VERSION");

pub type MachineLabels = BTreeMap<String, BTreeMap<String, String>>;

#[derive(Clone)]
pub struct BackendSink<C = HttpBackendClient> {
    client: C,
    machine_labels: Arc<MachineLabels>,
}

impl BackendSink<HttpBackendClient> {
    pub fn new(
        endpoint: impl Into<String>,
        jwt: impl Into<String>,
        machine_labels: MachineLabels,
    ) -> Result<Self, Box<dyn Error>> {
        Ok(Self::with_client(
            HttpBackendClient::new(endpoint, jwt)?,
            machine_labels,
        ))
    }

    /// Update the bearer token without rebuilding the underlying HTTP client.
    pub fn set_jwt(&mut self, jwt: impl Into<String>) {
        self.client.set_jwt(jwt);
    }
}

impl<C> BackendSink<C> {
    pub fn with_client(client: C, machine_labels: MachineLabels) -> Self {
        Self {
            client,
            machine_labels: Arc::new(machine_labels),
        }
    }
}

impl<C> BackendSink<C>
where
    C: BackendClient,
{
    /// Write a batch using the backend's status-only success contract.
    pub fn write_batch(&self, inventories: &[NodeInventory]) -> Result<(), Box<dyn Error>> {
        self.client
            .put_inventory(inventories, self.machine_labels.as_ref())
    }
}

impl<C> InventorySink for BackendSink<C>
where
    C: BackendClient,
{
    fn write(&self, inventories: &[NodeInventory]) -> Result<(), Box<dyn Error>> {
        self.write_batch(inventories)
    }
}

pub trait BackendClient {
    fn put_inventory(
        &self,
        inventories: &[NodeInventory],
        machine_labels: &MachineLabels,
    ) -> Result<(), Box<dyn Error>>;
}

#[derive(Clone)]
pub struct HttpBackendClient {
    endpoint: String,
    jwt: String,
    client: reqwest::blocking::Client,
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

    pub fn enroll(endpoint: &str, agent_token: &str) -> Result<String, Box<dyn Error>> {
        Self::enroll_with_timeout(
            endpoint,
            agent_token,
            Duration::from_secs(DEFAULT_BACKEND_TIMEOUT_SECONDS),
        )
    }

    pub fn enroll_with_timeout(
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
            .send()?;
        let status = response.status();
        if !status.is_success() {
            let server = response_header(&response, reqwest::header::SERVER.as_str());
            let request_id = response_header(&response, "x-request-id");
            let body = response.text().unwrap_or_default();
            return Err(Box::new(BackendHttpError::with_metadata(
                status.as_u16(),
                body,
                server,
                request_id,
            )));
        }
        let response: EnrollResponse = response.json()?;

        if response.jwt_assertion.trim().is_empty() {
            return Err("enrollment response missing jwtAssertion field".into());
        }

        Ok(response.jwt_assertion)
    }

    fn set_jwt(&mut self, jwt: impl Into<String>) {
        self.jwt = jwt.into();
    }
}

fn backend_http_client(
    request_timeout: Duration,
) -> Result<reqwest::blocking::Client, reqwest::Error> {
    reqwest::blocking::Client::builder()
        .timeout(request_timeout)
        .connect_timeout(
            request_timeout.min(Duration::from_secs(DEFAULT_BACKEND_CONNECT_TIMEOUT_SECONDS)),
        )
        .build()
}

fn backend_request(
    client: &reqwest::blocking::Client,
    method: reqwest::Method,
    endpoint: &str,
) -> reqwest::blocking::RequestBuilder {
    client
        .request(method, endpoint)
        .header(reqwest::header::USER_AGENT, BACKEND_USER_AGENT)
}

fn response_header(response: &reqwest::blocking::Response, name: &str) -> Option<String> {
    response
        .headers()
        .get(name)
        .and_then(|value| value.to_str().ok())
        .map(str::to_string)
}

impl BackendClient for HttpBackendClient {
    fn put_inventory(
        &self,
        inventories: &[NodeInventory],
        machine_labels: &MachineLabels,
    ) -> Result<(), Box<dyn Error>> {
        let response = backend_request(&self.client, reqwest::Method::PUT, &self.endpoint)
            .bearer_auth(&self.jwt)
            .json(&build_batch_upsert_request(inventories, machine_labels))
            .send()?;
        let status = response.status();
        if !status.is_success() {
            let server = response_header(&response, reqwest::header::SERVER.as_str());
            let request_id = response_header(&response, "x-request-id");
            let body = response.text().unwrap_or_default();
            return Err(Box::new(BackendHttpError::with_metadata(
                status.as_u16(),
                body,
                server,
                request_id,
            )));
        }

        // The backend contract is status-only: any 2xx response accepts the
        // complete request batch. Do not require or interpret a response body.
        Ok(())
    }
}

fn build_batch_upsert_request<'a>(
    inventories: &'a [NodeInventory],
    machine_labels: &'a MachineLabels,
) -> BatchUpsertRequest<'a> {
    let data = inventories
        .iter()
        .map(|inventory| {
            let labels = machine_labels.get(&inventory.source.mac.to_ascii_lowercase());
            NodeUpsertRequest {
                inventory: BackendInventory(inventory),
                compute_zone: labels.and_then(|labels| nonempty_label(labels, "compute_zone")),
                node_group: labels.and_then(|labels| nonempty_label(labels, "node_group")),
            }
        })
        .collect();
    BatchUpsertRequest {
        agent: AgentInfo {
            version: AGENT_VERSION,
        },
        data,
    }
}

fn nonempty_label<'a>(labels: &'a BTreeMap<String, String>, name: &str) -> Option<&'a str> {
    labels
        .get(name)
        .map(String::as_str)
        .filter(|value| !value.is_empty())
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
    #[serde(skip_serializing_if = "Option::is_none")]
    compute_zone: Option<&'a str>,
    #[serde(skip_serializing_if = "Option::is_none")]
    node_group: Option<&'a str>,
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
            uuid,
            source,
            primary_system_id,
            managers,
            systems,
            chassis,
            firmware,
            domain_errors,
            target_error,
        } = self.0;
        let mut map = serializer.serialize_map(Some(12))?;
        map.serialize_entry("schemaVersion", schema_version)?;
        map.serialize_entry("collectedAt", collected_at)?;
        map.serialize_entry("enrolledAt", enrolled_at)?;
        map.serialize_entry("uuid", uuid)?;
        map.serialize_entry("source", source)?;
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

    #[test]
    fn backend_sink_puts_node_inventory_batch() {
        let put_batches = Rc::new(RefCell::new(Vec::new()));
        let sink = BackendSink::with_client(
            RecordingClient {
                put_batches: Rc::clone(&put_batches),
            },
            MachineLabels::new(),
        );

        sink.write(&[inventory("10.0.0.1"), inventory("10.0.0.2")])
            .expect("write");

        let put_batches = put_batches.borrow();
        assert_eq!(put_batches.len(), 1);
        assert_eq!(put_batches[0].len(), 2);
        assert_eq!(put_batches[0][0], "10.0.0.1");
        assert_eq!(put_batches[0][1], "10.0.0.2");
    }

    #[test]
    fn batch_request_exports_placement_labels() {
        let inventory = inventory("10.0.0.1");
        let machine_labels = BTreeMap::from([(
            inventory.source.mac.to_ascii_lowercase(),
            BTreeMap::from([
                ("compute_zone".to_string(), "az51".to_string()),
                ("node_group".to_string(), "dev3-dh1".to_string()),
                ("oob".to_string(), "true".to_string()),
            ]),
        )]);
        let inventories = [inventory];
        let request = build_batch_upsert_request(&inventories, &machine_labels);

        let value = serde_json::to_value(request).expect("serialize request");
        assert_eq!(value["agent"]["version"], env!("CARGO_PKG_VERSION"));
        assert_eq!(value["data"][0]["source"]["address"], "10.0.0.1");
        assert_eq!(value["data"][0]["computeZone"], "az51");
        assert_eq!(value["data"][0]["nodeGroup"], "dev3-dh1");
    }

    #[test]
    fn batch_request_omits_missing_placement_labels() {
        let inventory = inventory("10.0.0.1");
        let inventories = [inventory];
        let machine_labels = MachineLabels::new();
        let request = build_batch_upsert_request(&inventories, &machine_labels);

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
        let machine_labels = MachineLabels::new();
        let request = build_batch_upsert_request(&inventories, &machine_labels);

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
    fn backend_sink_updates_jwt_without_rebuilding() {
        let mut sink = BackendSink::new(
            "https://backend.example.com/v1/agent/oob_nodes",
            "old-jwt",
            MachineLabels::new(),
        )
        .expect("sink");

        sink.set_jwt("new-jwt");

        assert_eq!(sink.client.jwt, "new-jwt");
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

    impl BackendClient for RecordingClient {
        fn put_inventory(
            &self,
            inventories: &[NodeInventory],
            _machine_labels: &MachineLabels,
        ) -> Result<(), Box<dyn Error>> {
            self.put_batches.borrow_mut().push(
                inventories
                    .iter()
                    .map(|inventory| inventory.source.address.clone())
                    .collect(),
            );
            Ok(())
        }
    }

    fn inventory(address: &str) -> NodeInventory {
        NodeInventory {
            schema_version: crate::models::INVENTORY_SCHEMA_VERSION.to_string(),
            collected_at: "2026-01-01T00:00:00Z".to_string(),
            enrolled_at: None,
            uuid: None,
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
