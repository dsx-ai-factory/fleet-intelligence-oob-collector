use std::error::Error;
use std::fmt;
use std::io::Read;
use std::time::Duration;

use crate::models::NodeInventory;

use super::InventorySink;

pub const DEFAULT_BACKEND_TIMEOUT_SECONDS: u64 = 30;
pub const DEFAULT_BACKEND_CONNECT_TIMEOUT_SECONDS: u64 = 10;
const BACKEND_USER_AGENT: &str = "fleet-intelligence-oob-collector";
const MAX_ERROR_BODY_PREVIEW_BYTES: usize = 1024;

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

        let client = backend_http_client(request_timeout)?;
        let response = backend_request(&client, reqwest::Method::POST, endpoint)
            .bearer_auth(agent_token)
            .send()?;
        let status = response.status();
        if !status.is_success() {
            let server = response_header(&response, reqwest::header::SERVER.as_str());
            let request_id = response_header(&response, "x-request-id");
            let content_length = response.content_length();
            let body = read_bounded_response_body(response);
            return Err(Box::new(BackendHttpError::with_metadata(
                status.as_u16(),
                body,
                content_length,
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

#[derive(Debug)]
struct BoundedBodyPreview {
    body: String,
    preview_bytes: usize,
    truncated: bool,
}

fn read_bounded_response_body(mut response: reqwest::blocking::Response) -> BoundedBodyPreview {
    read_bounded_body(&mut response)
}

fn read_bounded_body(reader: &mut impl Read) -> BoundedBodyPreview {
    let mut bytes = Vec::with_capacity(MAX_ERROR_BODY_PREVIEW_BYTES + 1);
    let read_failed = reader
        .take((MAX_ERROR_BODY_PREVIEW_BYTES + 1) as u64)
        .read_to_end(&mut bytes)
        .is_err();
    bounded_body_preview(bytes, read_failed)
}

fn bounded_body_preview(mut bytes: Vec<u8>, truncated: bool) -> BoundedBodyPreview {
    let truncated = truncated || bytes.len() > MAX_ERROR_BODY_PREVIEW_BYTES;
    bytes.truncate(MAX_ERROR_BODY_PREVIEW_BYTES);
    let preview_bytes = bytes.len();
    let body = String::from_utf8_lossy(&bytes)
        .chars()
        .map(|character| {
            if character.is_control() {
                ' '
            } else {
                character
            }
        })
        .collect();
    BoundedBodyPreview {
        body,
        preview_bytes,
        truncated,
    }
}

impl BackendClient for HttpBackendClient {
    fn put_inventory(&self, inventories: &[NodeInventory]) -> Result<(), Box<dyn Error>> {
        let response = backend_request(&self.client, reqwest::Method::PUT, &self.endpoint)
            .bearer_auth(&self.jwt)
            .json(&BatchUpsertRequest { data: inventories })
            .send()?;
        let status = response.status();
        if !status.is_success() {
            let server = response_header(&response, reqwest::header::SERVER.as_str());
            let request_id = response_header(&response, "x-request-id");
            let content_length = response.content_length();
            let body = read_bounded_response_body(response);
            return Err(Box::new(BackendHttpError::with_metadata(
                status.as_u16(),
                body,
                content_length,
                server,
                request_id,
            )));
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
    server: Option<String>,
    request_id: Option<String>,
    response_content_length: Option<u64>,
    response_preview_bytes: usize,
    body_preview: String,
    body_truncated: bool,
}

impl BackendHttpError {
    pub fn new(status: u16, body: impl Into<String>) -> Self {
        let body = body.into();
        let content_length = body.len() as u64;
        Self::with_metadata(
            status,
            bounded_body_preview(body.into_bytes(), false),
            Some(content_length),
            None,
            None,
        )
    }

    fn with_metadata(
        status: u16,
        body: BoundedBodyPreview,
        response_content_length: Option<u64>,
        server: Option<String>,
        request_id: Option<String>,
    ) -> Self {
        Self {
            status,
            server,
            request_id,
            response_content_length,
            response_preview_bytes: body.preview_bytes,
            body_preview: body.body,
            body_truncated: body.truncated,
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
        if let Some(content_length) = self.response_content_length {
            write!(f, " response_content_length={content_length}")?;
        }
        write!(f, " response_preview_bytes={}", self.response_preview_bytes)?;
        if !self.body_preview.trim().is_empty() {
            write!(f, " response_body={:?}", self.body_preview)?;
        }
        if self.body_truncated {
            write!(f, " response_body_truncated=true")?;
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

    #[test]
    fn backend_error_bounds_body_and_includes_response_metadata() {
        let body = format!(
            "first line\n{}",
            "x".repeat(MAX_ERROR_BODY_PREVIEW_BYTES + 1)
        );
        let error = BackendHttpError::with_metadata(
            403,
            bounded_body_preview(body.clone().into_bytes(), false),
            Some(body.len() as u64),
            Some("awselb/2.0".to_string()),
            Some("request-123".to_string()),
        );
        let message = error.to_string();

        assert!(message.contains("http_status=403"));
        assert!(message.contains("server=\"awselb/2.0\""));
        assert!(message.contains("request_id=\"request-123\""));
        assert!(message.contains(&format!("response_content_length={}", body.len())));
        assert!(message.contains(&format!(
            "response_preview_bytes={MAX_ERROR_BODY_PREVIEW_BYTES}"
        )));
        assert!(message.contains("response_body=\"first line "));
        assert!(message.contains("response_body_truncated=true"));
        assert!(!message.contains('\n'));
    }

    #[test]
    fn bounded_body_preview_stores_at_most_the_configured_limit() {
        let preview = bounded_body_preview(vec![b'x'; MAX_ERROR_BODY_PREVIEW_BYTES + 10], false);

        assert_eq!(preview.preview_bytes, MAX_ERROR_BODY_PREVIEW_BYTES);
        assert_eq!(preview.body.len(), MAX_ERROR_BODY_PREVIEW_BYTES);
        assert!(preview.truncated);
    }

    #[test]
    fn bounded_body_reader_consumes_only_limit_plus_one_byte() {
        let mut reader = std::io::Cursor::new(vec![b'x'; MAX_ERROR_BODY_PREVIEW_BYTES * 2]);

        let preview = read_bounded_body(&mut reader);

        assert_eq!(reader.position(), (MAX_ERROR_BODY_PREVIEW_BYTES + 1) as u64);
        assert_eq!(preview.preview_bytes, MAX_ERROR_BODY_PREVIEW_BYTES);
        assert!(preview.truncated);
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
