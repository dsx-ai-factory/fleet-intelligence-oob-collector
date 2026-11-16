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
use reqwest::header::{AUTHORIZATION, HeaderValue};
use serde::{Deserialize, de::DeserializeOwned};
use url::Url;

use super::{Endpoint, EndpointSnapshot, EndpointSource, EndpointSourceError};
use crate::models::TargetKind;

type Result<T> = std::result::Result<T, EndpointSourceError>;
const PAGE_SIZE: usize = 100;
const MAX_PAGES: usize = 1000;
const MAX_BODY_BYTES: usize = 8 * 1024 * 1024;

/// Non-secret NICo REST settings. The token file contains only an access token,
/// refreshed by the operator or an external credential service.
#[derive(Clone, Debug, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct NicoRestConfig {
    /// REST prefix including `/v2`, without organization or API-name segments.
    pub api_url: String,
    pub org: String,
    pub site_id: String,
    pub token_file: PathBuf,
    /// Current NICo uses `nico`; older deployments may use `carbide`.
    #[serde(default = "default_api_name")]
    pub api_name: String,
    /// Shared BMC credential profile applied to every discovered machine.
    #[serde(default)]
    pub credentials_profile: Option<String>,
    /// Whole-discovery budget, including all pages and reading the token file.
    #[serde(default = "default_timeout")]
    pub timeout_seconds: u64,
    /// Runtime-only fallback age since the last successful discovery (30 hours).
    #[serde(default = "default_max_stale")]
    pub max_stale_seconds: u64,
}

fn default_api_name() -> String {
    "nico".into()
}
fn default_max_stale() -> u64 {
    108_000
}

fn default_timeout() -> u64 {
    30
}

/// A scoped NICo API adapter. It never contacts a BMC or follows HTTP redirects.
pub struct NicoRestEndpointSource {
    config: NicoRestConfig,
    machines_url: Url,
    client: reqwest::Client,
    timeout: Duration,
}

impl NicoRestEndpointSource {
    /// Construct an HTTPS-only source. Relative token paths must be rooted at
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
        if config.org.trim().is_empty()
            || config.site_id.trim().is_empty()
            || config.org == "."
            || config.org == ".."
            || !matches!(config.api_name.as_str(), "nico" | "carbide")
            || !config.token_file.is_absolute()
            || !(1..=3600).contains(&config.timeout_seconds)
            || !(1..=172_800).contains(&config.max_stale_seconds)
            || config
                .credentials_profile
                .as_ref()
                .is_some_and(|name| name.trim().is_empty())
        {
            return Err("NICo requires organization, site ID, an absolute token_file, api_name nico/carbide, timeout_seconds in 1..=3600, and max_stale_seconds in 1..=172800; credential profile must not be blank".into());
        }
        url.path_segments_mut()
            .map_err(|_| "invalid NICo REST API prefix")?
            .pop_if_empty()
            .extend(["org", &config.org, &config.api_name, "machine"]);
        let timeout = Duration::from_secs(config.timeout_seconds);
        let client = client_builder(timeout)
            .https_only(true)
            .build()
            .map_err(|_| "cannot initialize NICo HTTPS client")?;
        Ok(Self {
            config,
            machines_url: url,
            client,
            timeout,
        })
    }

    async fn discover_inner(&self) -> Result<EndpointSnapshot> {
        let token = tokio::fs::read_to_string(&self.config.token_file)
            .await
            .map_err(|_| unavailable("cannot read NICo token_file"))?;
        let token = token.trim();
        if token.is_empty() || token.len() > 65536 {
            return Err(
                "NICo token_file must contain a nonempty access token of at most 64 KiB".into(),
            );
        }
        let mut authorization = HeaderValue::from_str(&format!("Bearer {token}"))
            .map_err(|_| "invalid NICo bearer token header")?;
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
                .map_err(|_| unavailable("NICo request failed; check connectivity and CA trust"))?;
            if !response.status().is_success() {
                let status = response.status();
                let message = format!(
                    "NICo returned HTTP {}; check token expiry, carbide scope, and provider/site permissions",
                    status.as_u16()
                );
                if status.is_server_error() || matches!(status.as_u16(), 401 | 403 | 429) {
                    return Err(unavailable(message));
                }
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

/// Discovery could not reach an authoritative result; runtime retries may recover.
#[derive(Debug)]
pub struct DiscoveryUnavailable(String);
impl std::fmt::Display for DiscoveryUnavailable {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.0)
    }
}
impl std::error::Error for DiscoveryUnavailable {}
pub(crate) fn unavailable(message: impl Into<String>) -> EndpointSourceError {
    Box::new(DiscoveryUnavailable(message.into()))
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
            .map_err(|_| unavailable("NICo endpoint discovery timed out"))?
    }
}

async fn read_json<T: DeserializeOwned>(mut response: reqwest::Response) -> Result<T> {
    let mut bytes = Vec::new();
    while let Some(chunk) = response
        .chunk()
        .await
        .map_err(|_| unavailable("cannot read NICo response"))?
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
    #[serde(default)]
    labels: Option<BTreeMap<String, String>>,
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
        let mut labels = self.labels.unwrap_or_default();
        let compute_zone = labels.remove("compute_zone");
        let node_group = labels.remove("node_group");
        Ok(Endpoint {
            source_reference: Some(super::safe_source_reference(&self.id)),
            kind: TargetKind::Compute,
            name: self.hostname.filter(|name| !name.trim().is_empty()),
            ip,
            port: None,
            mac,
            credentials_profile,
            compute_zone,
            node_group,
            labels,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicU64, Ordering};
    use tokio::io::{AsyncReadExt, AsyncWriteExt};

    struct TokenFile(PathBuf);
    impl TokenFile {
        fn new(token: &str) -> Self {
            static NEXT: AtomicU64 = AtomicU64::new(0);
            let path = std::env::temp_dir().join(format!(
                "fleetint-nico-token-{}-{}",
                std::process::id(),
                NEXT.fetch_add(1, Ordering::Relaxed)
            ));
            std::fs::write(&path, token).unwrap();
            Self(path)
        }
    }
    impl Drop for TokenFile {
        fn drop(&mut self) {
            let _ = std::fs::remove_file(&self.0);
        }
    }

    fn config(path: PathBuf) -> NicoRestConfig {
        NicoRestConfig {
            api_url: "https://nico.example.com/v2/".into(),
            org: "example".into(),
            site_id: "site-id".into(),
            token_file: path,
            api_name: default_api_name(),
            credentials_profile: Some("bmc".into()),
            timeout_seconds: 2,
            max_stale_seconds: default_max_stale(),
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
        TokenFile,
        tokio::task::JoinHandle<Vec<String>>,
    ) {
        let token = TokenFile::new("fixture-token");
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let mut source = NicoRestEndpointSource::new(config(token.0.clone())).unwrap();
        source.machines_url = Url::parse(&format!(
            "http://{}/v2/org/example/nico/machine",
            listener.local_addr().unwrap()
        ))
        .unwrap();
        source.client = client_builder(Duration::from_secs(2))
            .no_proxy()
            .build()
            .unwrap();
        let server = tokio::spawn(async move {
            let mut requests = Vec::new();
            for reply in replies {
                let (mut socket, _) = listener.accept().await.unwrap();
                let mut bytes = Vec::new();
                loop {
                    let mut buffer = [0; 4096];
                    let count = socket.read(&mut buffer).await.unwrap();
                    assert_ne!(count, 0);
                    bytes.extend_from_slice(&buffer[..count]);
                    if bytes.windows(4).any(|value| value == b"\r\n\r\n") {
                        break;
                    }
                }
                requests.push(String::from_utf8(bytes).unwrap());
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
    async fn reloads_bearer_file_on_each_discovery_and_rejects_empty_tokens() {
        let (source, token, server) = fixture(vec![
            page(1, 1, vec![machine(0)]),
            page(1, 1, vec![machine(0)]),
        ])
        .await;
        source.discover().await.unwrap();
        tokio::fs::write(&token.0, "rotated-token\n").await.unwrap();
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
                .contains("nonempty access token")
        );
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
            if matches!(scenario, "401" | "403" | "500") {
                assert!(error.is::<DiscoveryUnavailable>());
            }
            let error = error.to_string();
            assert!(error.contains(&expected), "{scenario}: {error}");
            assert!(!error.contains("sensitive"));
            assert!(!error.contains("fixture-token"));
            assert!(!error.contains("http://"));
            server.await.unwrap();
        }
    }

    #[tokio::test]
    async fn skips_unready_machines_and_maps_user_labels_without_losing_healthy_hosts() {
        let mut rows = vec![machine(0)];
        rows[0]["labels"] =
            serde_json::json!({"rack":"rack-12", "compute_zone":"zone-a", "node_group":"group-a"});
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
        assert_eq!(endpoints[0].labels["rack"], "rack-12");
        assert_eq!(endpoints[0].compute_zone.as_deref(), Some("zone-a"));
        assert_eq!(endpoints[0].node_group.as_deref(), Some("group-a"));
        assert!(!endpoints[0].labels.contains_key("compute_zone"));
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
        let token = TokenFile::new("fixture-token");
        let mut cfg = config(token.0.clone());
        cfg.org = "example/name".into();
        cfg.api_name = "carbide".into();
        let source = NicoRestEndpointSource::new(cfg.clone()).unwrap();
        assert_eq!(
            source.machines_url.path(),
            "/v2/org/example%2Fname/carbide/machine"
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

    #[tokio::test]
    async fn discovery_deadline_covers_an_unresponsive_server() {
        let token = TokenFile::new("fixture-token");
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let mut source = NicoRestEndpointSource::new(config(token.0.clone())).unwrap();
        source.machines_url =
            Url::parse(&format!("http://{}/", listener.local_addr().unwrap())).unwrap();
        source.client = client_builder(Duration::from_secs(2))
            .no_proxy()
            .build()
            .unwrap();
        source.timeout = Duration::from_millis(100);
        let error = source.discover().await.unwrap_err().to_string();
        assert!(error.contains("timed out"));
    }
}
