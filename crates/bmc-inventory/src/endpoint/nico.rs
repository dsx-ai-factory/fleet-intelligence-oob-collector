// Copyright (c) 2026, NVIDIA CORPORATION & AFFILIATES. All rights reserved.
// SPDX-License-Identifier: Apache-2.0

//! NICo REST machine discovery. BMC credentials and Fleet normalization are
//! resolved separately by the shared endpoint configuration.

use std::collections::{BTreeMap, HashMap, HashSet};
use std::net::IpAddr;
use std::path::PathBuf;
use std::time::{Duration, Instant};

use async_trait::async_trait;
use mac_address::MacAddress;
use oauth2::{
    AsyncHttpClient, AuthType, ClientId, ClientSecret, Scope, TokenResponse, TokenUrl,
    basic::BasicClient,
};
use reqwest::header::{AUTHORIZATION, HeaderValue};
use serde::{Deserialize, de::DeserializeOwned};
use url::Url;

use super::{Endpoint, EndpointSnapshot, EndpointSource, EndpointSourceError, PlacementOrigin};
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
    pub token_url: String,
    pub client_id_file: PathBuf,
    pub client_secret_file: PathBuf,
    /// Scopes requested from the identity provider; empty omits the scope parameter.
    #[serde(default)]
    pub scopes: Vec<String>,
    /// Required shared BMC credential profile applied to every discovered machine.
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
    sites_url: Url,
    token_url: Url,
    client: reqwest::Client,
    timeout: Duration,
}

impl NicoRestEndpointSource {
    /// Construct an HTTPS-only source. Relative credential paths must be rooted at
    /// the config directory by the caller before constructing this source.
    pub fn new(config: NicoRestConfig) -> Result<Self> {
        let mut url = https_url("NICo api_url", &config.api_url)?;
        let token_url = https_url("OAuth2 token_url", &config.token_url)?;
        if config.org.trim().is_empty()
            || config.org == "."
            || config.org == ".."
            || !config.client_id_file.is_absolute()
            || !config.client_secret_file.is_absolute()
            || !(1..=3600).contains(&config.timeout_seconds)
            || config
                .credentials_profile
                .as_ref()
                .is_none_or(|name| name.trim().is_empty())
        {
            return Err("NICo requires organization, absolute OAuth2 client credential paths, timeout_seconds in 1..=3600; a nonblank shared credentials_profile is required".into());
        }
        url.path_segments_mut()
            .map_err(|_| "invalid NICo REST API prefix")?
            .pop_if_empty()
            .extend(["org", &config.org, "nico", "machine"]);
        let sites_url = url.join("site").map_err(|_| "invalid NICo site URL")?;
        let timeout = Duration::from_secs(config.timeout_seconds);
        let client = client_builder(timeout)
            .https_only(true)
            .build()
            .map_err(|_| "cannot initialize NICo HTTPS client")?;
        Ok(Self {
            config,
            machines_url: url,
            sites_url,
            token_url,
            client,
            timeout,
        })
    }

    async fn discover_inner(&self) -> Result<EndpointSnapshot> {
        let started = Instant::now();
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
        // A failed listing is not retried here: every collection cycle
        // rediscovers, so the next cycle is the retry.
        self.discover_pages(&authorization, started).await
    }

    async fn list_pages<T: DeserializeOwned>(
        &self,
        url: &Url,
        authorization: &HeaderValue,
        query: &[(&str, String)],
    ) -> Result<Vec<T>> {
        let mut rows = Vec::new();
        let mut total = None;
        for page in 1..=MAX_PAGES {
            let response = self
                .client
                .get(url.clone())
                .header(AUTHORIZATION, authorization.clone())
                .query(query)
                .query(&[("pageNumber", page), ("pageSize", PAGE_SIZE)])
                .send()
                .await
                .map_err(|_| "NICo request failed; check connectivity and CA trust")?;
            if !response.status().is_success() {
                return Err(format!(
                    "NICo returned HTTP {}; check OAuth2 scopes and provider/site permissions",
                    response.status().as_u16()
                )
                .into());
            }
            let pagination: Pagination = response
                .headers()
                .get("x-pagination")
                .and_then(|value| value.to_str().ok())
                .and_then(|value| serde_json::from_str(value).ok())
                .ok_or("NICo response has no valid X-Pagination header")?;
            // NICo echoes the requested page and size; a different size, a
            // changed total, or a short page means the listing moved underneath us.
            if pagination.page_number != page
                || pagination.page_size != PAGE_SIZE
                || total.is_some_and(|previous| previous != pagination.total)
            {
                return Err(LISTING_CHANGED.into());
            }
            total = Some(pagination.total);
            let batch: Vec<T> = read_json(response).await?;
            let expected = pagination.total.saturating_sub(rows.len()).min(PAGE_SIZE);
            if batch.len() != expected {
                return Err(LISTING_CHANGED.into());
            }
            rows.extend(batch);
            if rows.len() == pagination.total {
                return Ok(rows);
            }
        }
        Err("NICo discovery exceeded the page limit".into())
    }

    async fn discover_pages(
        &self,
        authorization: &HeaderValue,
        started: Instant,
    ) -> Result<EndpointSnapshot> {
        let sites: Vec<Site> = self
            .list_pages(
                &self.sites_url,
                authorization,
                &[("orderBy", "NAME_ASC".into())],
            )
            .await?;
        // Sites only supply placement. The site list can include sites the
        // identity sees through a tenant role but cannot list machines for, so
        // machines are listed once at org scope, where NICo applies the caller's
        // own scope, and joined to their site by `siteId`.
        let mut placements = HashMap::new();
        for site in &sites {
            if site.id.trim().is_empty() {
                return Err("NICo site has an empty ID".into());
            }
            if placements
                .insert(site.id.as_str(), site.placement())
                .is_some()
            {
                return Err(LISTING_CHANGED.into());
            }
        }
        let machines: Vec<Machine> = self
            .list_pages(
                &self.machines_url,
                authorization,
                &[
                    ("includeMetadata", "true".into()),
                    ("orderBy", "ID_ASC".into()),
                ],
            )
            .await?;
        let mut endpoints = Vec::new();
        let mut ids = HashSet::new();
        let mut skipped = 0usize;
        let mut macs = HashMap::new();
        let mut addresses = HashMap::new();
        for machine in machines {
            let machine_id = super::safe_source_reference(&machine.id);
            if machine.id.trim().is_empty() {
                return Err("NICo machine has an empty ID".into());
            }
            if !ids.insert(machine.id.clone()) {
                return Err(LISTING_CHANGED.into());
            }
            // A machine on a site outside the site list keeps no site placement;
            // configured defaults can still fill it.
            let (compute_zone, node_group) = placements
                .get(machine.site_id.as_str())
                .cloned()
                .unwrap_or_default();
            let endpoint = machine
                .into_endpoint(self.config.credentials_profile.clone())
                .and_then(|mut endpoint| {
                    // An unusable hostname skips only this machine, before the
                    // record reaches the shared finalizer.
                    endpoint.name = super::normalize_value("name", endpoint.name.take())
                        .map_err(|error| error.to_string())?;
                    endpoint.compute_zone = compute_zone;
                    endpoint.node_group = node_group;
                    Ok(endpoint)
                });
            let endpoint = match endpoint {
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
        if skipped > 0 {
            tracing::warn!(
                skipped_machines = skipped,
                usable_machines = endpoints.len(),
                total_machines = ids.len(),
                "NICo discovery omitted machines; inspect metadata, names, and permissions (first 20 reasons logged)"
            );
        }
        if endpoints.is_empty() {
            return Err("no usable BMC targets found in NICo endpoint source; check sites, metadata and provider permissions".into());
        }
        tracing::info!(
            sites = sites.len(),
            machines = ids.len(),
            targets = endpoints.len(),
            skipped_machines = skipped,
            elapsed_ms = u64::try_from(started.elapsed().as_millis()).unwrap_or(u64::MAX),
            "NICo discovery completed"
        );
        Ok(endpoints.into())
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

const LISTING_CHANGED: &str =
    "NICo listing changed during discovery or paginated inconsistently; the next cycle retries";

// Parse an endpoint URL that may receive credentials or bearer tokens. Errors
// name only the setting, never the configured value.
fn https_url(setting: &str, value: &str) -> Result<Url> {
    let url = Url::parse(value).map_err(|_| format!("invalid {setting}"))?;
    if url.scheme() != "https"
        || url.host_str().is_none()
        || !url.username().is_empty()
        || url.password().is_some()
        || url.query().is_some()
        || url.fragment().is_some()
    {
        return Err(
            format!("{setting} must be HTTPS without credentials, query, or fragment").into(),
        );
    }
    Ok(url)
}

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
        .map_err(|_| "NICo response does not match the site/machine JSON contract".into())
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct Pagination {
    page_number: usize,
    page_size: usize,
    total: usize,
}

#[derive(Deserialize)]
struct Site {
    id: String,
    name: String,
    #[serde(default)]
    location: Option<SiteLocation>,
}

#[derive(Deserialize)]
struct SiteLocation {
    #[serde(default)]
    country: Option<String>,
    #[serde(default)]
    state: Option<String>,
    #[serde(default)]
    city: Option<String>,
}

impl Site {
    /// Site-derived compute zone and node group. Configured defaults override
    /// these during finalization. An unusable value is dropped with one warning,
    /// so it never removes this site's machines from the snapshot.
    fn placement(&self) -> (Option<String>, Option<String>) {
        let site_id = super::safe_source_reference(&self.id);
        let usable = |field, value| {
            super::normalize_value(field, value)
                .inspect_err(|error| {
                    tracing::warn!(%site_id, reason = %error, "ignoring NICo site placement");
                })
                .ok()
                .flatten()
        };
        (
            usable("compute_zone", self.compute_zone()),
            usable("node_group", Some(self.name.clone())),
        )
    }

    fn compute_zone(&self) -> Option<String> {
        let location = self.location.as_ref()?;
        let raw = [&location.country, &location.state, &location.city]
            .into_iter()
            .filter_map(|part| part.as_deref())
            .collect::<Vec<_>>()
            .join("-");
        let mut zone = String::new();
        for ch in raw.chars().flat_map(char::to_lowercase) {
            if ch.is_alphanumeric() {
                zone.push(ch);
            } else if !zone.is_empty() && !zone.ends_with('-') {
                zone.push('-');
            }
        }
        let zone = zone.trim_end_matches('-');
        (!zone.is_empty()).then(|| zone.to_string())
    }
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
            // `::ffff:a.b.c.d` is the IPv4 peer `a.b.c.d`. Canonicalize once so the
            // duplicate and placeholder checks and the Redfish address agree.
            .map(|ip| ip.to_canonical())
            // An unspecified address is a placeholder, never a reachable BMC.
            .filter(|ip| !ip.is_unspecified())
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
            placement_origin: PlacementOrigin::Discovered,
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

    // The exact decoded value of one query parameter in a recorded request line.
    fn query_value(request: &str, key: &str) -> String {
        let target = request.split(' ').nth(1).unwrap();
        let query = target.split_once('?').map_or("", |(_, query)| query);
        url::form_urlencoded::parse(query.as_bytes())
            .find(|(name, _)| name == key)
            .unwrap_or_else(|| panic!("{key} missing from {target}"))
            .1
            .into_owned()
    }

    fn site_page() -> Reply {
        page(
            1,
            1,
            vec![serde_json::json!({
                "id":"site-id", "name":"test-cluster",
                "location":{"country":"USA", "state":"AZ", "city":"Phoenix"}
            })],
        )
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
        fixture_with_sites(replies, token_reply, scopes, true).await
    }

    async fn fixture_with_sites(
        replies: Vec<Reply>,
        token_reply: Option<Reply>,
        scopes: Vec<String>,
        automatic_sites: bool,
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
        source.sites_url = source.machines_url.join("site").unwrap();
        let requested_scopes = source.config.scopes.join(" ");
        let server = tokio::spawn(async move {
            let mut requests = Vec::new();
            let mut replies = replies.into_iter().peekable();
            // After an OAuth2 redirect, keep listening briefly so a followed
            // redirect would be recorded instead of failing on a closed port.
            let mut watch_for_redirect = false;
            while watch_for_redirect || replies.peek().is_some() {
                let (mut socket, _) = if watch_for_redirect {
                    match tokio::time::timeout(Duration::from_millis(300), listener.accept()).await
                    {
                        Ok(accepted) => accepted.unwrap(),
                        Err(_) => break,
                    }
                } else {
                    listener.accept().await.unwrap()
                };
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
                    requests.push(request.clone());
                    if automatic_sites && request.starts_with("GET /v2/org/example/nico/site?") {
                        site_page()
                    } else {
                        replies.next().unwrap()
                    }
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
                if watch_for_redirect {
                    break;
                }
                if token_request && token_reply.is_some() {
                    if !(300..400).contains(&reply.status) {
                        break;
                    }
                    watch_for_redirect = true;
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
        let requests = server.await.unwrap();
        assert!(requests[0].starts_with("GET /v2/org/example/nico/site?"));
        assert!(requests[0].contains("orderBy=NAME_ASC"));
        for (index, request) in requests[1..].iter().enumerate() {
            assert!(request.starts_with("GET /v2/org/example/nico/machine?"));
            // Org scope: NICo applies the caller's own scope, never a listed site.
            assert!(!request.contains("siteId="));
            for (key, value) in [
                ("includeMetadata", "true"),
                ("pageSize", "100"),
                ("orderBy", "ID_ASC"),
            ] {
                assert_eq!(query_value(request, key), value);
            }
            assert_eq!(query_value(request, "pageNumber"), (index + 1).to_string());
            assert!(!request.contains("isMissingOnSite="));
            assert!(!request.contains("status="));
            assert!(
                request
                    .to_lowercase()
                    .contains("authorization: bearer fixture-token")
            );
        }
    }

    #[tokio::test]
    async fn site_placement_is_marked_as_discovered_so_configured_defaults_win() {
        // Precedence itself is applied by the shared finalizer; see
        // config::targets tests for configured-over-discovered placement.
        let (source, _files, server) = fixture(vec![page(1, 1, vec![machine(0)])]).await;
        let endpoints = source.discover().await.unwrap();
        assert_eq!(endpoints[0].compute_zone.as_deref(), Some("usa-az-phoenix"));
        assert_eq!(endpoints[0].node_group.as_deref(), Some("test-cluster"));
        assert_eq!(endpoints[0].placement_origin, PlacementOrigin::Discovered);
        server.await.unwrap();
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
        assert!(requests[2].contains("Bearer rotated-token"));
        assert!(!requests[2].contains("fixture-token"));
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
        for scopes in [
            vec![],
            vec!["carbide".into()],
            vec!["inventory.read".into(), "site.read".into()],
        ] {
            let (source, _files, server) =
                fixture_with_oauth2(vec![page(1, 1, vec![machine(0)])], None, scopes).await;
            assert_eq!(source.discover().await.unwrap().len(), 1);
            assert_eq!(server.await.unwrap().len(), 2);
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
                    "listing changed".into()
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
                    "listing changed".into()
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
    async fn skips_invalid_and_decommissioned_machines_and_ignores_machine_labels() {
        let mut rows = vec![machine(0)];
        rows[0]["status"] = "FutureLifecycleState".into();
        rows[0]["isMissingOnSite"] = true.into();
        rows[0]["labels"] = serde_json::json!({
            "fleet-role": "NSV Device",
            "rack": "rack-12",
            "compute_zone": "zone-a",
            "node_group": "group-a"
        });
        for index in 1..=9 {
            let mut row = machine(index);
            match index {
                1 => row["metadata"] = serde_json::Value::Null,
                2 => row["metadata"]["bmcInfo"] = serde_json::Value::Null,
                3 => row["metadata"]["bmcInfo"]["ip"] = serde_json::Value::Null,
                4 => row["metadata"]["bmcInfo"]["mac"] = "invalid".into(),
                5 => row["metadata"]["bmcInfo"]["ip"] = "invalid".into(),
                6 => row["metadata"]["bmcInfo"]["ip"] = "0.0.0.0".into(),
                7 => row["metadata"]["bmcInfo"]["ip"] = "::".into(),
                8 => row["metadata"]["bmcInfo"]["ip"] = "::ffff:0.0.0.0".into(),
                _ => row["status"] = "Decommissioned".into(),
            }
            rows.push(row);
        }
        rows[0]["metadata"]["bmcInfo"]["ip"] = "::ffff:192.0.2.1".into();
        let (source, _token, server) = fixture(vec![page(1, rows.len(), rows)]).await;
        let endpoints = source.discover().await.unwrap();
        assert_eq!(endpoints.len(), 1);
        // IPv4-mapped addresses are stored, and sent to Redfish, as plain IPv4.
        assert_eq!(endpoints[0].address(), "192.0.2.1");
        assert!(endpoints[0].labels.is_empty());
        assert_eq!(endpoints[0].compute_zone.as_deref(), Some("usa-az-phoenix"));
        assert_eq!(endpoints[0].node_group.as_deref(), Some("test-cluster"));
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
    async fn paginates_sites_and_joins_machines_to_their_site() {
        let sites: Vec<_> = (0..101)
            .map(|index| {
                serde_json::json!({
                    "id":format!("site-{index}"), "name":format!(" cluster-{index} "),
                    "location":{"country":"USA", "state":"AZ", "city":"Phoenix"}
                })
            })
            .collect();
        let machines: Vec<_> = (0..101)
            .map(|index| {
                let mut row = machine(index);
                row["siteId"] = format!("site-{index}").into();
                row
            })
            .collect();
        let replies = vec![
            page(1, 101, sites[..100].to_vec()),
            page(2, 101, sites[100..].to_vec()),
            page(1, 101, machines[..100].to_vec()),
            page(2, 101, machines[100..].to_vec()),
        ];
        let (source, _files, server) = fixture_with_sites(replies, None, vec![], false).await;
        let endpoints = source.discover().await.unwrap();
        assert_eq!(endpoints.len(), 101);
        for (index, endpoint) in endpoints.iter().enumerate() {
            assert_eq!(
                endpoint.node_group.as_deref(),
                Some(format!("cluster-{index}").as_str())
            );
            assert_eq!(endpoint.compute_zone.as_deref(), Some("usa-az-phoenix"));
        }
        let requests = server.await.unwrap();
        assert_eq!(requests.len(), 4);
        assert!(requests[1].contains("/site?orderBy=NAME_ASC&pageNumber=2"));
        assert_eq!(query_value(&requests[3], "pageNumber"), "2");
    }

    #[tokio::test]
    async fn sites_without_machine_access_do_not_block_discovery() {
        // A dual-role identity's site list can include a tenant-visible site owned
        // by another provider; NICo returns 403 for machines filtered by that
        // site. Listing machines at org scope never asks for it. A machine whose
        // site is missing from the list keeps no site placement.
        let sites = page(
            1,
            3,
            vec![
                serde_json::json!({"id":"foreign-site", "name":"tenant-visible"}),
                serde_json::json!({"id":"site-id", "name":"own-cluster"}),
                serde_json::json!({"id":"empty", "name":"empty-cluster"}),
            ],
        );
        let mut unlisted = machine(1);
        unlisted["siteId"] = "unlisted-site".into();
        let (source, _files, server) = fixture_with_sites(
            vec![sites, page(1, 2, vec![machine(0), unlisted])],
            None,
            vec![],
            false,
        )
        .await;
        let endpoints = source.discover().await.unwrap();
        let groups: Vec<_> = endpoints.iter().map(|e| e.node_group.as_deref()).collect();
        assert_eq!(groups, [Some("own-cluster"), None]);
        let requests = server.await.unwrap();
        assert_eq!(requests.len(), 2);
        assert!(!requests[1].contains("siteId="));
    }

    #[tokio::test]
    async fn machine_listing_failure_aborts_the_snapshot() {
        let failure = Reply {
            status: 403,
            pagination: None,
            body: "sensitive-site-response".into(),
            extra_header: String::new(),
        };
        let (source, _files, server) = fixture(vec![failure]).await;
        let error = source.discover().await.unwrap_err().to_string();
        assert!(error.contains("HTTP 403"));
        assert!(!error.contains("sensitive"));
        assert_eq!(server.await.unwrap().len(), 2);
    }

    #[tokio::test]
    async fn rejects_invalid_and_duplicate_site_listings() {
        // Each case fails on the first site page with no retry and no machine query.
        for (rows, expected_error) in [
            (
                vec![serde_json::json!({"id":"", "name":"cluster"})],
                "empty ID",
            ),
            (
                vec![
                    serde_json::json!({"id":"site-id", "name":"one"}),
                    serde_json::json!({"id":"site-id", "name":"two"}),
                ],
                "listing changed",
            ),
        ] {
            let replies = vec![page(1, rows.len(), rows)];
            let (source, _files, server) = fixture_with_sites(replies, None, vec![], false).await;
            let error = source.discover().await.unwrap_err().to_string();
            assert!(error.contains(expected_error), "{error}");
            let requests = tokio::time::timeout(Duration::from_secs(5), server)
                .await
                .expect("fixture expected another site request")
                .unwrap();
            assert_eq!(requests.len(), 1);
        }
    }

    #[test]
    fn normalizes_location_and_handles_missing_components_without_inventing_a_zone() {
        for (location, expected) in [
            (
                serde_json::json!({"country":" USA ", "state":"AZ", "city":"Phoenix"}),
                Some("usa-az-phoenix"),
            ),
            (
                serde_json::json!({"country":"USA", "city":"San  José / West"}),
                Some("usa-san-josé-west"),
            ),
            (
                serde_json::json!({"country":null, "state":"AZ"}),
                Some("az"),
            ),
            (serde_json::json!({"city":" --- "}), None),
            (serde_json::json!({}), None),
            (serde_json::Value::Null, None),
        ] {
            let site: Site = serde_json::from_value(
                serde_json::json!({"id":"site", "name":"cluster", "location":location}),
            )
            .unwrap();
            assert_eq!(site.compute_zone().as_deref(), expected);
        }
    }

    #[tokio::test]
    async fn identity_conflicts_name_both_machines_and_never_return_a_partial_list() {
        // The second machine repeats the first's MAC, IP, or IPv4-mapped IP.
        for (key, value) in [
            ("mac", "02:00:00:00:00:00"),
            ("ip", "192.0.2.1"),
            ("ip", "::ffff:192.0.2.1"),
        ] {
            let first = machine(0);
            let mut second = machine(1);
            second["metadata"]["bmcInfo"][key] = value.into();
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
        cfg = config(token.0.clone());
        cfg.timeout_seconds = 0;
        assert!(NicoRestEndpointSource::new(cfg).is_err());
        cfg = config(PathBuf::from("relative-token"));
        assert!(NicoRestEndpointSource::new(cfg).is_err());
    }

    #[test]
    fn requires_a_shared_bmc_profile_before_discovery() {
        let files = CredentialFiles::new("fixture-secret");
        for profile in [None, Some("".into()), Some("   ".into())] {
            let mut cfg = config(files.0.clone());
            cfg.credentials_profile = profile;
            let error = NicoRestEndpointSource::new(cfg).err().unwrap().to_string();
            assert!(error.contains("credentials_profile is required"), "{error}");
        }
    }

    #[tokio::test]
    async fn bad_hostnames_skip_machines_and_bad_site_placement_is_dropped() {
        let sites = page(
            1,
            3,
            vec![
                serde_json::json!({"id":"site-id", "name":"good-cluster"}),
                serde_json::json!({"id":"long-site", "name":"é".repeat(513)}),
                serde_json::json!({"id":"blank-site", "name":" "}),
            ],
        );
        let mut long_host = machine(1);
        long_host["hostname"] = "é".repeat(513).into();
        let mut long_site_host = machine(2);
        long_site_host["siteId"] = "long-site".into();
        let mut blank_site_host = machine(3);
        blank_site_host["siteId"] = "blank-site".into();
        let replies = vec![
            sites,
            page(
                1,
                4,
                vec![machine(0), long_host, long_site_host, blank_site_host],
            ),
        ];
        let (source, _files, server) = fixture_with_sites(replies, None, vec![], false).await;
        let endpoints = source.discover().await.unwrap();
        // The long hostname skips only machine 1. Unusable site names leave the
        // node group unset, which configured defaults can still fill.
        let names: Vec<_> = endpoints.iter().map(|e| e.name.as_deref()).collect();
        assert_eq!(names, [Some("host-0"), Some("host-2"), Some("host-3")]);
        let groups: Vec<_> = endpoints.iter().map(|e| e.node_group.as_deref()).collect();
        assert_eq!(groups, [Some("good-cluster"), None, None]);
        server.await.unwrap();
    }

    #[tokio::test]
    async fn rejects_a_page_size_other_than_requested() {
        // NICo echoes the requested pageSize, so any other value is inconsistent.
        for size in [0, 1, 101] {
            let mut reply = site_page();
            reply.pagination =
                Some(serde_json::json!({"pageNumber":1,"pageSize":size,"total":1}).to_string());
            let (source, _files, server) =
                fixture_with_sites(vec![reply], None, vec![], false).await;
            let error = source.discover().await.unwrap_err().to_string();
            assert!(error.contains("listing changed"), "{size}: {error}");
            assert_eq!(server.await.unwrap().len(), 1);
        }
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
