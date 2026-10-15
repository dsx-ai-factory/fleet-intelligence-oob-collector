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
use std::net::SocketAddr;
use std::sync::Arc;
use std::time::Duration;

use async_trait::async_trait;
use bmc_dispatch::{Dispatcher, DispatchingHttpClient};
use nv_redfish::ServiceRoot;
use nv_redfish::bmc_http::{BmcCredentials, CacheSettings, HttpBmc};
use url::Url;

use super::resources::collect_report_from_root;
use crate::config::Target;
use crate::models::{LatencyMeasurement, NodeInventory};
use crate::source::assembly::assemble;
use crate::source::tcp_probe::{self, LatencyProbe};
use crate::source::{InventorySource, RedfishReport, now_rfc3339};

type RedfishBmc = HttpBmc<DispatchingHttpClient<nv_redfish::bmc_http::reqwest::Client>>;

const MAX_REDIRECTS: usize = 3;
/// Identifies collector traffic in BMC access logs.
const USER_AGENT: &str = "fleet-intelligence-oob-collector";
const DEFAULT_REDFISH_PORT: u16 = 443;

pub struct RedfishSource {
    client: nv_redfish::bmc_http::reqwest::Client,
    dispatcher: Arc<Dispatcher>,
    latency_probe: LatencyProbe,
}

impl RedfishSource {
    pub fn new(
        insecure_tls: bool,
        request_timeout: Duration,
        dispatcher: Arc<Dispatcher>,
    ) -> Result<Self, Box<dyn Error>> {
        if request_timeout.is_zero() {
            return Err("request timeout must be greater than zero".into());
        }
        let connect_timeout = request_timeout.min(Duration::from_secs(10));
        let client = nv_redfish::bmc_http::reqwest::Client::with_params(
            nv_redfish::bmc_http::reqwest::ClientParams::new()
                .timeout(request_timeout)
                .connect_timeout(connect_timeout)
                .accept_invalid_certs(insecure_tls)
                .user_agent(USER_AGENT)
                // Some BMCs canonicalize the service root (for example,
                // /redfish/v1 -> /redfish/v1/). Keep redirects bounded while
                // allowing those standards-compatible endpoints to work.
                .max_redirects(MAX_REDIRECTS),
        )?;
        Ok(Self {
            client,
            dispatcher,
            latency_probe: LatencyProbe::Disabled,
        })
    }

    /// Measure collector-to-BMC latency after each target's Redfish
    /// collection.
    #[must_use]
    pub fn with_latency_probe(mut self, latency_probe: LatencyProbe) -> Self {
        self.latency_probe = latency_probe;
        self
    }

    async fn measure_latency(&self, target: &Target) -> Vec<LatencyMeasurement> {
        let destination = SocketAddr::new(target.ip, target.port.unwrap_or(DEFAULT_REDFISH_PORT));
        let config = match self.latency_probe {
            LatencyProbe::Disabled => return vec![tcp_probe::disabled(destination)],
            LatencyProbe::Intercepted => return vec![tcp_probe::intercepted(destination)],
            LatencyProbe::Enabled(config) => config,
        };
        let Some(gate) = self.dispatcher.inventory_gate(&target.mac) else {
            return vec![tcp_probe::unavailable(
                destination,
                format!("no dispatcher endpoint for target {:?}", target.mac),
            )];
        };
        // One admission covers the whole round so the probe respects per-BMC
        // rate and concurrency limits without spending a token per connect.
        let measurement = gate
            .execute(async {
                Ok::<_, std::convert::Infallible>(tcp_probe::probe(destination, config).await)
            })
            .await
            .unwrap_or_else(|error| tcp_probe::unavailable(destination, error.to_string()));
        vec![measurement]
    }

    async fn collect_report(&self, target: &Target) -> Result<RedfishReport, String> {
        let root = self
            .service_root(target)
            .await
            .map_err(|err| err.to_string())?;
        collect_report_from_root(&root).await
    }

    async fn service_root(
        &self,
        target: &Target,
    ) -> Result<ServiceRoot<RedfishBmc>, Box<dyn Error>> {
        let url = bmc_url(&target.address)?;
        let gate = self
            .dispatcher
            .inventory_gate(&target.mac)
            .ok_or_else(|| format!("no dispatcher endpoint for target {:?}", target.mac))?;
        let bmc = Arc::new(HttpBmc::new(
            DispatchingHttpClient::new(self.client.clone(), gate),
            url,
            BmcCredentials::new(target.username.clone(), target.password.clone()),
            CacheSettings::with_capacity(32),
        ));
        Ok(ServiceRoot::new(bmc).await?)
    }
}

#[async_trait]
impl InventorySource for RedfishSource {
    async fn collect(&self, target: Target) -> NodeInventory {
        let collected_at = now_rfc3339();
        let result = self.collect_report(&target).await;
        // Probe only targets whose Redfish collection completed.
        let latency = if result.is_ok() {
            self.measure_latency(&target).await
        } else {
            Vec::new()
        };
        assemble(target, collected_at, result, latency)
    }
}

fn bmc_url(address: &str) -> Result<Url, Box<dyn Error>> {
    let url: Url = if address.starts_with("http://") || address.starts_with("https://") {
        address.parse()?
    } else {
        format!("https://{address}").parse()?
    };
    if url.scheme() == "http" {
        tracing::warn!(
            address = %url,
            "BMC target is configured with http://; Redfish credentials and inventory will not use TLS"
        );
    }
    Ok(url)
}

#[cfg(test)]
mod tests {
    use super::*;
    use bmc_dispatch::DispatcherConfig;

    fn test_dispatcher() -> Dispatcher {
        Dispatcher::start(
            DispatcherConfig {
                global_max_in_flight: 1,
                bmc_max_in_flight: 1,
                bmc_requests_per_second: 100,
                bmc_burst: 100,
                workflow_queue_capacity: 8,
            },
            ["test".to_string()],
        )
        .expect("dispatcher")
    }

    #[tokio::test]
    async fn redfish_failure_skips_latency_probe() {
        let listener = std::net::TcpListener::bind("127.0.0.1:0").expect("bind");
        let port = listener.local_addr().expect("addr").port();
        drop(listener);
        let source = RedfishSource::new(true, Duration::from_secs(5), Arc::new(test_dispatcher()))
            .expect("source")
            .with_latency_probe(LatencyProbe::Enabled(tcp_probe::TcpProbeConfig::default()));
        let target = Target {
            kind: crate::models::TargetKind::Compute,
            name: None,
            compute_zone: None,
            node_group: None,
            ip: "127.0.0.1".parse().expect("ip"),
            port: Some(port),
            mac: "test".to_string(),
            address: format!("127.0.0.1:{port}"),
            username: "user".to_string(),
            password: "password".to_string(),
            labels: Default::default(),
        };

        let inventory = source.collect(target).await;

        assert!(inventory.target_error.is_some());
        assert!(inventory.latency.is_empty());
    }

    #[tokio::test]
    async fn disabled_probe_reports_disabled_status() {
        let source = RedfishSource::new(true, Duration::from_secs(5), Arc::new(test_dispatcher()))
            .expect("source")
            .with_latency_probe(LatencyProbe::Disabled);
        let target = Target {
            kind: crate::models::TargetKind::Compute,
            name: None,
            compute_zone: None,
            node_group: None,
            ip: "10.0.0.1".parse().expect("ip"),
            port: None,
            mac: "test".to_string(),
            address: "10.0.0.1".to_string(),
            username: "user".to_string(),
            password: "password".to_string(),
            labels: Default::default(),
        };

        let latency = source.measure_latency(&target).await;

        assert_eq!(latency.len(), 1);
        assert_eq!(latency[0].status, crate::models::LatencyStatus::Disabled);
        assert_eq!(latency[0].destination, "10.0.0.1:443");
        assert_eq!(latency[0].attempts, 0);
        assert!(latency[0].min_us.is_none());
        assert_eq!(
            serde_json::to_value(&latency[0]).expect("serialize")["status"],
            "disabled"
        );
    }

    #[tokio::test]
    async fn redfish_source_builds_client() {
        RedfishSource::new(true, Duration::from_secs(30), Arc::new(test_dispatcher()))
            .expect("source");
    }

    #[tokio::test]
    async fn redfish_source_rejects_zero_timeout() {
        let Err(error) = RedfishSource::new(true, Duration::ZERO, Arc::new(test_dispatcher()))
        else {
            panic!("zero timeout should fail");
        };

        assert!(!error.to_string().is_empty());
    }

    #[test]
    fn bmc_url_adds_https_to_bare_address() {
        let url = bmc_url("10.0.0.1").expect("url");

        assert_eq!(url.as_str(), "https://10.0.0.1/");
        assert_eq!(url.port_or_known_default(), Some(443));
    }

    #[test]
    fn bmc_url_keeps_explicit_port() {
        let url = bmc_url("https://bmc.example.com:8443").expect("url");

        assert_eq!(url.as_str(), "https://bmc.example.com:8443/");
    }

    #[test]
    fn bmc_url_uses_scheme_default_port_for_scheme_only_url() {
        let url = bmc_url("http://bmc.example.com").expect("url");

        assert_eq!(url.as_str(), "http://bmc.example.com/");
        assert_eq!(url.port_or_known_default(), Some(80));
    }

    #[test]
    fn bmc_url_rejects_invalid_explicit_url() {
        let error = bmc_url("https://").expect_err("invalid url");

        assert!(!error.to_string().is_empty());
    }
}
