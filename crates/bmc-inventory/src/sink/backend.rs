use std::error::Error;
use std::fmt;
use std::time::Duration;

use crate::models::NodeInventory;

use super::InventorySink;

pub const DEFAULT_BACKEND_TIMEOUT_SECONDS: u64 = 30;
pub const DEFAULT_BACKEND_CONNECT_TIMEOUT_SECONDS: u64 = 10;

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
}

impl<C> BackendSink<C> {
    pub fn with_client(client: C) -> Self {
        Self { client }
    }
}

impl<C> InventorySink for BackendSink<C>
where
    C: BackendClient,
{
    fn write(&self, inventories: &[NodeInventory]) -> Result<(), Box<dyn Error>> {
        self.client.put_inventory(inventories)
    }
}

pub trait BackendClient {
    fn put_inventory(&self, inventories: &[NodeInventory]) -> Result<(), Box<dyn Error>>;
}

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

        let response = backend_http_client(request_timeout)?
            .post(endpoint)
            .bearer_auth(agent_token)
            .send()?
            .error_for_status()?;
        let response: EnrollResponse = response.json()?;

        if response.jwt_assertion.trim().is_empty() {
            return Err("enrollment response missing jwtAssertion field".into());
        }

        Ok(response.jwt_assertion)
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

impl BackendClient for HttpBackendClient {
    fn put_inventory(&self, inventories: &[NodeInventory]) -> Result<(), Box<dyn Error>> {
        let response = self
            .client
            .put(&self.endpoint)
            .bearer_auth(&self.jwt)
            .json(&BatchUpsertRequest { data: inventories })
            .send()?;
        let status = response.status();
        if !status.is_success() {
            let body = response.text().unwrap_or_default();
            return Err(Box::new(BackendHttpError::new(status.as_u16(), body)));
        }

        let response: BatchUpsertResponse = response.json()?;

        let failures: Vec<_> = response
            .results
            .into_iter()
            .filter(|result| result.status != "OK")
            .collect();
        if !failures.is_empty() {
            return Err(Box::new(BackendUpsertError { failures }));
        }

        Ok(())
    }
}

#[derive(Debug)]
pub struct BackendHttpError {
    status: u16,
    body: String,
}

impl BackendHttpError {
    pub fn new(status: u16, body: impl Into<String>) -> Self {
        Self {
            status,
            body: body.into(),
        }
    }

    pub fn status(&self) -> u16 {
        self.status
    }
}

impl fmt::Display for BackendHttpError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        if self.body.trim().is_empty() {
            write!(f, "backend request failed with HTTP {}", self.status)
        } else {
            write!(
                f,
                "backend request failed with HTTP {}: {}",
                self.status, self.body
            )
        }
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
    data: &'a [NodeInventory],
}

#[derive(Debug, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
struct BatchUpsertResponse {
    results: Vec<NodeUpsertResult>,
}

#[derive(Debug, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
struct NodeUpsertResult {
    node_uuid: Option<String>,
    status: String,
    message: Option<String>,
}

#[derive(Debug)]
struct BackendUpsertError {
    failures: Vec<NodeUpsertResult>,
}

impl fmt::Display for BackendUpsertError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "backend rejected {} inventory item(s)",
            self.failures.len()
        )?;
        for failure in self.failures.iter().take(3) {
            write!(
                f,
                "; nodeUUID={} status={} message={}",
                failure.node_uuid.as_deref().unwrap_or("<missing>"),
                failure.status,
                failure.message.as_deref().unwrap_or("")
            )?;
        }
        if self.failures.len() > 3 {
            write!(f, "; ...")?;
        }
        Ok(())
    }
}

impl Error for BackendUpsertError {}

#[cfg(test)]
mod tests {
    use std::cell::RefCell;
    use std::rc::Rc;

    use super::*;

    #[test]
    fn backend_sink_puts_node_inventory_batch() {
        let put_batches = Rc::new(RefCell::new(Vec::new()));
        let sink = BackendSink::with_client(RecordingClient {
            put_batches: Rc::clone(&put_batches),
        });

        sink.write(&[inventory("10.0.0.1"), inventory("10.0.0.2")])
            .expect("write");

        let put_batches = put_batches.borrow();
        assert_eq!(put_batches.len(), 1);
        assert_eq!(put_batches[0].len(), 2);
        assert_eq!(put_batches[0][0], "10.0.0.1");
        assert_eq!(put_batches[0][1], "10.0.0.2");
    }

    #[test]
    fn backend_error_summarizes_failed_items() {
        let err = BackendUpsertError {
            failures: vec![NodeUpsertResult {
                node_uuid: None,
                status: "MISSING_UUID".to_string(),
                message: Some("item missing top-level uuid".to_string()),
            }],
        };

        assert_eq!(
            err.to_string(),
            "backend rejected 1 inventory item(s); nodeUUID=<missing> status=MISSING_UUID message=item missing top-level uuid"
        );
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

    #[derive(Clone)]
    struct RecordingClient {
        put_batches: Rc<RefCell<Vec<Vec<String>>>>,
    }

    impl BackendClient for RecordingClient {
        fn put_inventory(&self, inventories: &[NodeInventory]) -> Result<(), Box<dyn Error>> {
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
            uuid: None,
            source: crate::models::InventorySourceInfo {
                source_type: "redfish".to_string(),
                address: address.to_string(),
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
