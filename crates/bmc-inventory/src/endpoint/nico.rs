// Copyright (c) 2026, NVIDIA CORPORATION & AFFILIATES. All rights reserved.
// SPDX-License-Identifier: Apache-2.0

//! NICo REST machine discovery. BMC credentials and Fleet normalization are
//! resolved separately by the shared endpoint configuration.

use std::collections::{BTreeMap, HashMap, HashSet};
use std::net::IpAddr;
use std::path::PathBuf;
use std::time::Duration;

use async_trait::async_trait;
use mac_address::MacAddress;
use oauth2::{
    AsyncHttpClient, AuthType, ClientId, ClientSecret, Scope, TokenResponse, TokenUrl,
    basic::BasicClient,
};
use reqwest::header::{AUTHORIZATION, HeaderValue};
use serde::{Deserialize, de::DeserializeOwned};
use url::Url;

use super::{Endpoint, EndpointSnapshot, EndpointSource, EndpointSourceError};
use crate::models::TargetKind;

type Result<T> = std::result::Result<T, EndpointSourceError>;
const PAGE_SIZE: usize = 100;
const MAX_PAGES: usize = 1000;
const MAX_BODY_BYTES: usize = 8 * 1024 * 1024;

/// NICo REST settings and paths to mounted OAuth2 client credential files.
#[derive(Clone, Debug, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct NicoRestConfig {
    /// REST prefix including `/v2`, without organization or resource segments.
    pub api_url: String,
    pub org: String,
    pub site_id: String,
    pub token_url: String,
    pub client_id_file: PathBuf,
    pub client_secret_file: PathBuf,
    /// Scopes requested from the identity provider; empty omits the scope parameter.
    #[serde(default)]
    pub scopes: Vec<String>,
    /// Shared BMC credential profile applied to every discovered machine.
    #[serde(default)]
    pub credentials_profile: Option<String>,
    /// Whole-discovery budget, including credential loading, OAuth2, and all pages.
    #[serde(default = "default_timeout")]
    pub timeout_seconds: u64,
}

fn default_timeout() -> u64 {
    30
}

/// A scoped NICo API adapter. It never contacts a BMC or follows HTTP redirects.
pub struct NicoRestEndpointSource {
    config: NicoRestConfig,
    machines_url: Url,
    token_url: Url,
    client: reqwest::Client,
    timeout: Duration,
}

impl NicoRestEndpointSource {
    /// Construct an HTTPS-only source. Relative credential paths must be rooted at
    /// the config directory by the caller before constructing this source.
    pub fn new(config: NicoRestConfig) -> Result<Self> {
        let mut url = Url::parse(&config.api_url).map_err(|_| "invalid NICo REST API URL")?;
        if url.scheme() != "https"
            || url.host_str().is_none()
            || !url.username().is_empty()
            || url.password().is_some()
            || url.query().is_some()
            || url.fragment().is_some()
        {
            return Err(
                "NICo api_url must be HTTPS without credentials, query, or fragment".into(),
            );
        }
        let token_url = Url::parse(&config.token_url).map_err(|_| "invalid OAuth2 token URL")?;
        if token_url.scheme() != "https"
            || token_url.host_str().is_none()
            || !token_url.username().is_empty()
            || token_url.password().is_some()
            || token_url.query().is_some()
            || token_url.fragment().is_some()
        {
            return Err(
                "OAuth2 token_url must be HTTPS without credentials, query, or fragment".into(),
            );
        }
        if config.org.trim().is_empty()
            || config.site_id.trim().is_empty()
            || config.org == "."
            || config.org == ".."
            || !config.client_id_file.is_absolute()
            || !config.client_secret_file.is_absolute()
            || !(1..=3600).contains(&config.timeout_seconds)
            || config
                .credentials_profile
                .as_ref()
                .is_some_and(|name| name.trim().is_empty())
        {
            return Err("NICo requires organization, site ID, absolute OAuth2 client credential paths, timeout_seconds in 1..=3600; credential profile must not be blank".into());
        }
        if config.scopes.iter().any(|scope| {
            scope.is_empty()
                || scope
                    .bytes()
                    .any(|byte| !matches!(byte, 0x21 | 0x23..=0x5b | 0x5d..=0x7e))
        }) {
            return Err("OAuth2 scopes must be nonempty scope names without whitespace, quotes, or backslashes".into());
        }
        url.path_segments_mut()
            .map_err(|_| "invalid NICo REST API prefix")?
            .pop_if_empty()
            .extend(["org", &config.org, "nico", "machine"]);
        let timeout = Duration::from_secs(config.timeout_seconds);
        let client = client_builder(timeout)
            .https_only(true)
            .build()
            .map_err(|_| "cannot initialize NICo HTTPS client")?;
        Ok(Self {
            config,
            machines_url: url,
            token_url,
            client,
            timeout,
        })
    }

    async fn discover_inner(&self) -> Result<EndpointSnapshot> {
        // Read mounted Secret files for every exchange. A new discovery never
        // reuses credentials or a bearer token across a Vault/VSO rotation.
        let client_id = read_client_credential(&self.config.client_id_file).await?;
        let client_secret = read_client_credential(&self.config.client_secret_file).await?;
        let oauth = BasicClient::new(ClientId::new(client_id))
            .set_client_secret(ClientSecret::new(client_secret))
            .set_auth_type(AuthType::BasicAuth)
            .set_token_uri(TokenUrl::from_url(self.token_url.clone()));
        let transport = OAuth2HttpClient(self.client.clone());
        let token = oauth.exchange_client_credentials()
            .add_scopes(self.config.scopes.iter().cloned().map(Scope::new))
            .request_async(&transport).await
            // OAuth errors may contain the response body or URLs. Never expose
            // provider diagnostics that could echo credentials or access tokens.
            .map_err(|_| "OAuth2 client-credentials exchange failed; check client credentials, token URL, CA trust, and configured scopes")?;
        if token.token_type() != &oauth2::basic::BasicTokenType::Bearer {
            return Err("OAuth2 token endpoint must issue a Bearer access token".into());
        }
        let access_token = token.access_token().secret();
        if access_token.is_empty() || access_token.len() > 65536 {
            return Err(
                "OAuth2 response must contain a nonempty access_token of at most 64 KiB".into(),
            );
        }
        let mut authorization = HeaderValue::from_str(&format!("Bearer {access_token}"))
            .map_err(|_| "invalid OAuth2 access token header")?;
        authorization.set_sensitive(true);
        for attempt in 0..2 {
            match self.discover_pages(&authorization).await {
                Err(error) if attempt == 0 && error.is::<ListingChanged>() => {
                    tracing::warn!(
                        "NICo listing changed; retrying discovery once within the original deadline"
                    );
                }
                result => return result,
            }
        }
        unreachable!("both discovery attempts return or retry")
    }

    async fn discover_pages(&self, authorization: &HeaderValue) -> Result<EndpointSnapshot> {
        let mut endpoints = Vec::new();
        let mut ids = HashSet::new();
        let mut total = None;
        let mut skipped = 0usize;
        let mut macs = HashMap::new();
        let mut addresses = HashMap::new();
        for page in 1..=MAX_PAGES {
            let response = self
                .client
                .get(self.machines_url.clone())
                .header(AUTHORIZATION, authorization.clone())
                .query(&[
                    ("siteId", self.config.site_id.clone()),
                    ("includeMetadata", "true".into()),
                    ("pageNumber", page.to_string()),
                    ("pageSize", PAGE_SIZE.to_string()),
                    ("orderBy", "ID_ASC".into()),
                    ("isMissingOnSite", "false".into()),
                    ("status", "Ready".into()),
                    ("status", "InUse".into()),
                    ("status", "Initializing".into()),
                    ("status", "Error".into()),
                    ("status", "Maintenance".into()),
                    ("status", "Reset".into()),
                    ("status", "Unknown".into()),
                ])
                .send()
                .await
                .map_err(|_| "NICo request failed; check connectivity and CA trust")?;
            if !response.status().is_success() {
                let status = response.status();
                let message = format!(
                    "NICo returned HTTP {}; check OAuth2 scopes and provider/site permissions",
                    status.as_u16()
                );
                return Err(message.into());
            }
            let pagination: Pagination = response
                .headers()
                .get("x-pagination")
                .and_then(|value| value.to_str().ok())
                .and_then(|value| serde_json::from_str(value).ok())
                .ok_or("NICo response has no valid X-Pagination header")?;
            if pagination.page_number != page
                || pagination.page_size != PAGE_SIZE
                || total.is_some_and(|previous| previous != pagination.total)
            {
                return Err(Box::new(ListingChanged));
            }
            total = Some(pagination.total);
            let machines: Vec<Machine> = read_json(response).await?;
            let expected = pagination
                .total
                .saturating_sub((page - 1) * PAGE_SIZE)
                .min(PAGE_SIZE);
            if machines.len() != expected {
                return Err(Box::new(ListingChanged));
            }
            for machine in machines {
                let machine_id = super::safe_source_reference(&machine.id);
                if machine.site_id != self.config.site_id || machine.id.trim().is_empty() {
                    return Err(
                        format!("NICo machine {machine_id}: different site or empty ID").into(),
                    );
                }
                if !ids.insert(machine.id.clone()) {
                    return Err(Box::new(ListingChanged));
                }
                let endpoint = match machine.into_endpoint(self.config.credentials_profile.clone())
                {
                    Ok(endpoint) => endpoint,
                    Err(error) => {
                        skipped += 1;
                        if skipped <= 20 {
                            tracing::warn!(%machine_id, reason = %error, "skipping NICo machine");
                        }
                        continue;
                    }
                };
                if let Some(previous) = macs.insert(endpoint.mac, machine_id.clone()) {
                    return Err(format!(
                        "NICo machines {previous} and {machine_id}: duplicate BMC MAC {}",
                        endpoint.mac
                    )
                    .into());
                }
                if let Some(previous) = addresses.insert(endpoint.ip, machine_id.clone()) {
                    return Err(format!(
                        "NICo machines {previous} and {machine_id}: duplicate BMC address"
                    )
                    .into());
                }
                endpoints.push(endpoint);
            }
            if ids.len() == pagination.total {
                if skipped > 0 {
                    tracing::warn!(
                        skipped_machines = skipped,
                        usable_machines = endpoints.len(),
                        total_machines = ids.len(),
                        "NICo discovery omitted machines; inspect metadata and permissions (first 20 reasons logged)"
                    );
                }
                if endpoints.is_empty() {
                    return Err("no usable BMC targets found in NICo endpoint source; check metadata and provider permissions".into());
                }
                return Ok(endpoints.into());
            }
        }
        Err("NICo discovery exceeded the page limit".into())
    }
}

// Keep OAuth protocol handling in oauth2, while applying the same bounded,
// secret-safe HTTP policy as the discovery client.
struct OAuth2HttpClient(reqwest::Client);
impl<'a> AsyncHttpClient<'a> for OAuth2HttpClient {
    type Error = std::io::Error;
    type Future = std::pin::Pin<
        Box<
            dyn std::future::Future<Output = std::result::Result<oauth2::HttpResponse, Self::Error>>
                + Send
                + 'a,
        >,
    >;

    fn call(&'a self, mut request: oauth2::HttpRequest) -> Self::Future {
        Box::pin(async move {
            if let Some(header) = request.headers_mut().get_mut(AUTHORIZATION) {
                header.set_sensitive(true);
            }
            let request = reqwest::Request::try_from(request).map_err(std::io::Error::other)?;
            let mut response = self
                .0
                .execute(request)
                .await
                .map_err(std::io::Error::other)?;
            let mut builder = oauth2::http::Response::builder().status(response.status());
            *builder.headers_mut().expect("response builder headers") = response.headers().clone();
            let mut body = Vec::new();
            while let Some(chunk) = response.chunk().await.map_err(std::io::Error::other)? {
                if chunk.len() > (128 * 1024usize).saturating_sub(body.len()) {
                    return Err(std::io::Error::other("OAuth2 response exceeds 128 KiB"));
                }
                body.extend_from_slice(&chunk);
            }
            builder.body(body).map_err(std::io::Error::other)
        })
    }
}

async fn read_client_credential(path: &std::path::Path) -> Result<String> {
    let value = tokio::fs::read_to_string(path)
        .await
        .map_err(|_| "cannot read OAuth2 client credential file")?;
    let value = value.trim();
    if value.is_empty() || value.len() > 65536 || value.contains(['\r', '\n']) {
        return Err(
            "OAuth2 client credential files must contain a nonempty single value of at most 64 KiB"
                .into(),
        );
    }
    Ok(value.into())
}

#[derive(Debug)]
struct ListingChanged;
impl std::fmt::Display for ListingChanged {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("NICo pagination changed or is inconsistent after retry; rerun discovery")
    }
}
impl std::error::Error for ListingChanged {}

fn client_builder(timeout: Duration) -> reqwest::ClientBuilder {
    reqwest::Client::builder()
        .redirect(reqwest::redirect::Policy::none())
        .timeout(timeout)
        .connect_timeout(timeout.min(Duration::from_secs(10)))
        .user_agent(concat!("fleetint-oob-nico/", env!("CARGO_PKG_VERSION")))
}

#[async_trait]
impl EndpointSource for NicoRestEndpointSource {
    async fn discover(&self) -> Result<EndpointSnapshot> {
        tokio::time::timeout(self.timeout, self.discover_inner())
            .await
            .map_err(|_| "NICo endpoint discovery timed out")?
    }
}

async fn read_json<T: DeserializeOwned>(mut response: reqwest::Response) -> Result<T> {
    let mut bytes = Vec::new();
    while let Some(chunk) = response
        .chunk()
        .await
        .map_err(|_| "cannot read NICo response")?
    {
        if chunk.len() > MAX_BODY_BYTES.saturating_sub(bytes.len()) {
            return Err("NICo response exceeds 8 MiB per page".into());
        }
        bytes.extend_from_slice(&chunk);
    }
    serde_json::from_slice(&bytes)
        .map_err(|_| "NICo response does not match the machine JSON contract".into())
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct Pagination {
    page_number: usize,
    page_size: usize,
    total: usize,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct Machine {
    id: String,
    site_id: String,
    #[serde(default)]
    hostname: Option<String>,
    #[serde(default)]
    metadata: Option<Metadata>,
    #[serde(default)]
    status: Option<String>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct Metadata {
    bmc_info: Option<BmcInfo>,
}

#[derive(Deserialize)]
struct BmcInfo {
    ip: Option<String>,
    mac: Option<String>,
}

impl Machine {
    fn into_endpoint(self, credentials_profile: Option<String>) -> Result<Endpoint> {
        if self
            .status
            .as_deref()
            .is_some_and(|status| matches!(status, "Decommissioned" | "Decommissioning"))
        {
            return Err("machine is decommissioned or decommissioning".into());
        }
        let bmc = self.metadata.and_then(|metadata| metadata.bmc_info).ok_or(
            "NICo machine has no BMC metadata; provider access and includeMetadata are required",
        )?;
        let ip = bmc
            .ip
            .and_then(|ip| ip.parse::<IpAddr>().ok())
            .ok_or("NICo machine has no valid BMC IP")?;
        let mac = bmc
            .mac
            .and_then(|mac| mac.parse::<MacAddress>().ok())
            .ok_or("NICo machine has no valid BMC MAC")?;
        Ok(Endpoint {
            kind: TargetKind::Compute,
            name: self.hostname.filter(|name| !name.trim().is_empty()),
            ip,
            port: None,
            mac,
            credentials_profile,
            compute_zone: None,
            node_group: None,
            labels: BTreeMap::new(),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicU64, Ordering};
    use tokio::io::{AsyncReadExt, AsyncWriteExt};

    struct CredentialFiles(PathBuf);
    impl CredentialFiles {
        fn new(token: &str) -> Self {
            static NEXT: AtomicU64 = AtomicU64::new(0);
            let path = std::env::temp_dir().join(format!(
                "fleetint-nico-token-{}-{}",
                std::process::id(),
                NEXT.fetch_add(1, Ordering::Relaxed)
            ));
            std::fs::write(&path, token).unwrap();
            std::fs::write(path.with_extension("id"), "fixture-client").unwrap();
            Self(path)
        }
    }
    impl Drop for CredentialFiles {
        fn drop(&mut self) {
            let _ = std::fs::remove_file(&self.0);
            let _ = std::fs::remove_file(self.0.with_extension("id"));
        }
    }

    fn config(path: PathBuf) -> NicoRestConfig {
        NicoRestConfig {
            api_url: "https://nico.example.com/v2/".into(),
            org: "example".into(),
            site_id: "site-id".into(),
            token_url: "https://idp.example.com/token".into(),
            client_id_file: path.with_extension("id"),
            client_secret_file: path,
            scopes: vec!["test-scope".into()],
            credentials_profile: Some("bmc".into()),
            timeout_seconds: 2,
        }
    }

    fn machine(index: usize) -> serde_json::Value {
        serde_json::json!({
            "id":format!("machine-{index}"),"siteId":"site-id","hostname":format!("host-{index}"),
            "metadata":{"bmcInfo":{"ip":format!("192.0.2.{}",index + 1),"mac":format!("02:00:00:00:00:{index:02x}")}}
        })
    }

    #[derive(Clone)]
    struct Reply {
        status: u16,
        pagination: Option<String>,
        body: String,
        extra_header: String,
    }
    fn page(number: usize, total: usize, rows: Vec<serde_json::Value>) -> Reply {
        Reply {
            status: 200,
            pagination: Some(
                serde_json::json!({"pageNumber":number,"pageSize":100,"total":total}).to_string(),
            ),
            body: serde_json::to_string(&rows).unwrap(),
            extra_header: String::new(),
        }
    }

    // Local HTTP fixtures exercise requests/decoding. The production constructor
    // enforces HTTPS; only this private test setup permits loopback HTTP.
    async fn fixture(
        replies: Vec<Reply>,
    ) -> (
        NicoRestEndpointSource,
        CredentialFiles,
        tokio::task::JoinHandle<Vec<String>>,
    ) {
        fixture_with_oauth2(replies, None, vec!["test-scope".into()]).await
    }

    async fn fixture_with_oauth2(
        replies: Vec<Reply>,
        token_reply: Option<Reply>,
        scopes: Vec<String>,
    ) -> (
        NicoRestEndpointSource,
        CredentialFiles,
        tokio::task::JoinHandle<Vec<String>>,
    ) {
        let token = CredentialFiles::new("fixture-secret");
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let mut source = NicoRestEndpointSource::new(config(token.0.clone())).unwrap();
        source.config.scopes = scopes;
        source.machines_url = Url::parse(&format!(
            "http://{}/v2/org/example/nico/machine",
            listener.local_addr().unwrap()
        ))
        .unwrap();
        source.token_url = source.machines_url.join("/token").unwrap();
        source.client = client_builder(Duration::from_secs(2))
            .no_proxy()
            .build()
            .unwrap();
        let requested_scopes = source.config.scopes.join(" ");
        let server = tokio::spawn(async move {
            let mut requests = Vec::new();
            let mut replies = replies.into_iter().peekable();
            while replies.peek().is_some() {
                let (mut socket, _) = listener.accept().await.unwrap();
                let mut bytes = Vec::new();
                loop {
                    let mut buffer = [0; 4096];
                    let count = socket.read(&mut buffer).await.unwrap();
                    assert_ne!(count, 0);
                    bytes.extend_from_slice(&buffer[..count]);
                    if let Some(header_end) =
                        bytes.windows(4).position(|value| value == b"\r\n\r\n")
                    {
                        let headers = String::from_utf8_lossy(&bytes[..header_end]);
                        let length = headers
                            .lines()
                            .filter_map(|line| line.split_once(':'))
                            .find(|(name, _)| name.eq_ignore_ascii_case("content-length"))
                            .map(|(_, value)| value.trim().parse::<usize>().unwrap())
                            .unwrap_or(0);
                        if bytes.len() >= header_end + 4 + length {
                            break;
                        }
                    }
                }
                let request = String::from_utf8(bytes).unwrap();
                let token_request = request.starts_with("POST /token ");
                let reply = if token_request {
                    let (headers, body) = request.split_once("\r\n\r\n").unwrap();
                    let header = headers
                        .lines()
                        .find(|line| line.to_ascii_lowercase().starts_with("authorization:"))
                        .unwrap();
                    assert!(
                        header.contains("Basic Zml4dHVyZS1jbGllbnQ6Zml4dHVyZS1zZWNyZXQ=")
                            || header.contains("Basic cm90YXRlZC1jbGllbnQ6cm90YXRlZC1zZWNyZXQ="),
                        "unexpected OAuth2 authentication"
                    );
                    let parameters: std::collections::BTreeMap<_, _> =
                        url::form_urlencoded::parse(body.as_bytes()).collect();
                    assert_eq!(parameters["grant_type"], "client_credentials");
                    if requested_scopes.is_empty() {
                        assert_eq!(parameters.len(), 1);
                        assert!(!parameters.contains_key("scope"));
                    } else {
                        assert_eq!(parameters.len(), 2);
                        assert_eq!(parameters["scope"], requested_scopes);
                    }
                    token_reply.clone().unwrap_or_else(|| Reply {
                        status: 200, pagination: None, extra_header: String::new(),
                        body: serde_json::json!({"access_token": if header.contains("cm90YXRlZC1jbGllbnQ6cm90YXRlZC1zZWNyZXQ=") {"rotated-token"} else {"fixture-token"}, "token_type": "Bearer", "expires_in": 900}).to_string(),
                    })
                } else {
                    assert!(!request.contains("Basic "));
                    requests.push(request);
                    replies.next().unwrap()
                };
                let pagination = reply
                    .pagination
                    .map(|value| format!("X-Pagination: {value}\r\n"))
                    .unwrap_or_default();
                let response = format!(
                    "HTTP/1.1 {} Test\r\nContent-Type: application/json\r\n{pagination}{}Content-Length: {}\r\nConnection: close\r\n\r\n{}",
                    reply.status,
                    reply.extra_header,
                    reply.body.len(),
                    reply.body
                );
                // A bounded-body reader may stop before an oversized reply ends.
                let _ = socket.write_all(response.as_bytes()).await;
                if token_request && token_reply.is_some() {
                    break;
                }
            }
            requests
        });
        (source, token, server)
    }

    #[tokio::test]
    async fn pages_map_hosts_and_send_scoped_authenticated_requests() {
        let (source, _token, server) = fixture(vec![
            page(1, 101, (0..100).map(machine).collect()),
            page(2, 101, vec![machine(100)]),
        ])
        .await;
        let provider: &dyn EndpointSource = &source;
        let endpoints = provider.discover().await.unwrap();
        assert_eq!(endpoints.len(), 101);
        assert_eq!(endpoints[100].name.as_deref(), Some("host-100"));
        assert_eq!(endpoints[100].mac.to_string(), "02:00:00:00:00:64");
        assert_eq!(endpoints[0].kind, TargetKind::Compute);
        assert_eq!(endpoints[0].port, None);
        assert_eq!(endpoints[0].credentials_profile.as_deref(), Some("bmc"));
        assert!(endpoints[0].labels.is_empty());
        for (index, request) in server.await.unwrap().iter().enumerate() {
            assert!(request.starts_with("GET /v2/org/example/nico/machine?"));
            for value in [
                "siteId=site-id",
                "includeMetadata=true",
                "pageSize=100",
                "orderBy=ID_ASC",
                "isMissingOnSite=false",
                "status=Ready",
                "status=InUse",
            ] {
                assert!(request.contains(value));
            }
            assert!(request.contains(&format!("pageNumber={}", index + 1)));
            assert!(
                request
                    .to_lowercase()
                    .contains("authorization: bearer fixture-token")
            );
        }
    }

    #[tokio::test]
    async fn reloads_oauth2_credentials_on_each_discovery_and_rejects_empty_secrets() {
        let (source, token, server) = fixture(vec![
            page(1, 1, vec![machine(0)]),
            page(1, 1, vec![machine(0)]),
        ])
        .await;
        source.discover().await.unwrap();
        tokio::fs::write(&token.0, "rotated-secret\n")
            .await
            .unwrap();
        tokio::fs::write(token.0.with_extension("id"), "rotated-client\n")
            .await
            .unwrap();
        source.discover().await.unwrap();
        let requests = server.await.unwrap();
        assert!(requests[1].contains("Bearer rotated-token"));
        assert!(!requests[1].contains("fixture-token"));
        tokio::fs::write(&token.0, " \n ").await.unwrap();
        assert!(
            source
                .discover()
                .await
                .unwrap_err()
                .to_string()
                .contains("OAuth2 client credential files")
        );
    }

    #[tokio::test]
    async fn oauth2_scopes_are_configurable_and_optional() {
        for scopes in [vec![], vec!["inventory.read".into(), "site.read".into()]] {
            let (source, _files, server) =
                fixture_with_oauth2(vec![page(1, 1, vec![machine(0)])], None, scopes).await;
            assert_eq!(source.discover().await.unwrap().len(), 1);
            assert_eq!(server.await.unwrap().len(), 1);
        }
    }

    #[tokio::test]
    async fn oauth2_failures_do_not_query_nico_or_expose_credentials() {
        for (status, body) in [
            (
                400,
                r#"{"error":"invalid_scope","error_description":"sensitive fixture-secret"}"#
                    .to_string(),
            ),
            (
                401,
                r#"{"error":"invalid_client","error_description":"sensitive fixture-secret"}"#
                    .into(),
            ),
            (500, "sensitive fixture-secret".into()),
            (302, "sensitive fixture-secret".into()),
            (200, "not JSON: sensitive fixture-secret".into()),
            (200, r#"{"access_token":"","token_type":"Bearer"}"#.into()),
            (
                200,
                r#"{"access_token":"sensitive","token_type":"MAC"}"#.into(),
            ),
            (200, "x".repeat(128 * 1024 + 1)),
        ] {
            let reply = Reply {
                status,
                body,
                pagination: None,
                extra_header: "Location: /must-not-follow\r\n".into(),
            };
            let (source, _files, server) = fixture_with_oauth2(
                vec![page(1, 1, vec![machine(0)])],
                Some(reply),
                vec!["test-scope".into()],
            )
            .await;
            let error = source.discover().await.unwrap_err().to_string();
            assert!(error.contains("OAuth2"), "{error}");
            for sensitive in ["fixture-secret", "fixture-client", "sensitive", "http://"] {
                assert!(!error.contains(sensitive), "{error}");
            }
            assert!(
                server.await.unwrap().is_empty(),
                "no NICo query after failed OAuth2 exchange"
            );
        }
    }

    #[tokio::test]
    async fn errors_never_return_partial_pages_or_expose_response_bodies() {
        for scenario in [
            "401",
            "403",
            "500",
            "redirect",
            "short-page",
            "changed-total",
            "duplicate-id",
            "wrong-site",
            "missing-pagination",
            "bad-json",
            "empty",
            "oversized",
        ] {
            let mut reply = page(1, 1, vec![machine(0)]);
            let mut prefix = Vec::new();
            let expected = match scenario {
                "401" | "403" | "500" => {
                    reply.status = scenario.parse().unwrap();
                    reply.body = "sensitive-upstream-message".into();
                    format!("HTTP {scenario}")
                }
                "redirect" => {
                    reply.status = 302;
                    reply.extra_header = "Location: /elsewhere\r\n".into();
                    "HTTP 302".into()
                }
                "short-page" => {
                    reply.body = "[]".into();
                    "pagination changed".into()
                }
                "changed-total" | "duplicate-id" => {
                    prefix.push(page(1, 101, (0..100).map(machine).collect()));
                    reply = page(
                        2,
                        if scenario == "changed-total" {
                            102
                        } else {
                            101
                        },
                        vec![machine(0)],
                    );
                    "pagination changed".into()
                }
                "wrong-site" => {
                    let mut row = machine(0);
                    row["siteId"] = "other-site".into();
                    reply = page(1, 1, vec![row]);
                    "different site".into()
                }
                "missing-pagination" => {
                    reply.pagination = None;
                    "X-Pagination".into()
                }
                "bad-json" => {
                    reply.body = "sensitive-invalid-json".into();
                    "JSON contract".into()
                }
                "empty" => {
                    reply = page(1, 0, vec![]);
                    "no usable BMC targets".into()
                }
                _ => {
                    reply.body = "x".repeat(MAX_BODY_BYTES + 1);
                    "8 MiB".into()
                }
            };
            prefix.push(reply);
            if matches!(scenario, "short-page" | "changed-total" | "duplicate-id") {
                prefix.extend(prefix.clone());
            }
            let (source, _token, server) = fixture(prefix).await;
            let error = source.discover().await.unwrap_err();
            let error = error.to_string();
            assert!(error.contains(&expected), "{scenario}: {error}");
            assert!(!error.contains("sensitive"));
            assert!(!error.contains("fixture-token"));
            assert!(!error.contains("http://"));
            server.await.unwrap();
        }
    }

    #[tokio::test]
    async fn skips_unready_machines_and_ignores_machine_labels() {
        let mut rows = vec![machine(0)];
        rows[0]["labels"] = serde_json::json!({
            "fleet-role": "NSV Device",
            "rack": "rack-12",
            "compute_zone": "zone-a",
            "node_group": "group-a"
        });
        for index in 1..=6 {
            let mut row = machine(index);
            match index {
                1 => row["metadata"] = serde_json::Value::Null,
                2 => row["metadata"]["bmcInfo"] = serde_json::Value::Null,
                3 => row["metadata"]["bmcInfo"]["ip"] = serde_json::Value::Null,
                4 => row["metadata"]["bmcInfo"]["mac"] = "invalid".into(),
                5 => row["metadata"]["bmcInfo"]["ip"] = "invalid".into(),
                _ => row["status"] = "Decommissioned".into(),
            }
            rows.push(row);
        }
        let (source, _token, server) = fixture(vec![page(1, rows.len(), rows)]).await;
        let endpoints = source.discover().await.unwrap();
        assert_eq!(endpoints.len(), 1);
        assert!(endpoints[0].labels.is_empty());
        assert!(endpoints[0].compute_zone.is_none());
        assert!(endpoints[0].node_group.is_none());
        server.await.unwrap();
        let mut invalid = machine(1);
        invalid["metadata"] = serde_json::Value::Null;
        let (source, _token, server) = fixture(vec![page(1, 1, vec![invalid])]).await;
        assert!(
            source
                .discover()
                .await
                .unwrap_err()
                .to_string()
                .contains("no usable BMC targets")
        );
        server.await.unwrap();
    }

    #[tokio::test]
    async fn retries_one_complete_listing_after_pagination_changes() {
        let (source, _token, server) =
            fixture(vec![page(1, 1, vec![]), page(1, 1, vec![machine(0)])]).await;
        assert_eq!(source.discover().await.unwrap().len(), 1);
        let requests = server.await.unwrap();
        assert_eq!(requests.len(), 2);
        assert!(
            requests
                .iter()
                .all(|request| request.contains("pageNumber=1"))
        );
    }

    #[tokio::test]
    async fn identity_conflicts_name_both_machines_and_never_return_a_partial_list() {
        for duplicate_mac in [true, false] {
            let first = machine(0);
            let mut second = machine(1);
            let key = if duplicate_mac { "mac" } else { "ip" };
            second["metadata"]["bmcInfo"][key] = first["metadata"]["bmcInfo"][key].clone();
            let (source, _token, server) = fixture(vec![page(1, 2, vec![first, second])]).await;
            let error = source.discover().await.unwrap_err().to_string();
            assert!(error.contains("machine-0") && error.contains("machine-1"));
            assert!(!error.contains("fixture-token"));
            server.await.unwrap();
        }
    }

    #[test]
    fn validates_secure_settings_and_keeps_org_in_one_path_segment() {
        let token = CredentialFiles::new("fixture-secret");
        let mut cfg = config(token.0.clone());
        cfg.org = "example/name".into();
        let source = NicoRestEndpointSource::new(cfg.clone()).unwrap();
        assert_eq!(
            source.machines_url.path(),
            "/v2/org/example%2Fname/nico/machine"
        );
        for url in [
            "http://nico.example.com/v2",
            "https://user:secret@nico.example.com/v2",
            "https://nico.example.com/v2?token=secret",
            "https://nico.example.com/v2#fragment",
        ] {
            cfg.api_url = url.into();
            let error = NicoRestEndpointSource::new(cfg.clone())
                .err()
                .unwrap()
                .to_string();
            assert!(!error.contains("secret"));
        }
        for invalid_scope in ["", "two scopes", "quote\"", "backslash\\", "non-ascii-ü"] {
            cfg = config(token.0.clone());
            cfg.scopes = vec![invalid_scope.into()];
            assert!(NicoRestEndpointSource::new(cfg).is_err());
        }
        cfg = config(token.0.clone());
        cfg.timeout_seconds = 0;
        assert!(NicoRestEndpointSource::new(cfg).is_err());
        cfg = config(PathBuf::from("relative-token"));
        assert!(NicoRestEndpointSource::new(cfg).is_err());
    }

    #[tokio::test]
    async fn discovery_deadline_covers_an_unresponsive_server() {
        let token = CredentialFiles::new("fixture-secret");
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let mut source = NicoRestEndpointSource::new(config(token.0.clone())).unwrap();
        source.machines_url =
            Url::parse(&format!("http://{}/", listener.local_addr().unwrap())).unwrap();
        source.token_url = source.machines_url.join("/token").unwrap();
        source.client = client_builder(Duration::from_secs(2))
            .no_proxy()
            .build()
            .unwrap();
        source.timeout = Duration::from_millis(100);
        let error = source.discover().await.unwrap_err().to_string();
        assert!(error.contains("timed out"));
    }
}
