use std::error::Error;
use std::sync::Arc;
use std::time::Duration;

use async_trait::async_trait;
use bmc_dispatch::{Dispatcher, DispatchingHttpClient};
use nv_redfish::ServiceRoot;
use nv_redfish::bmc_http::{BmcCredentials, CacheSettings, HttpBmc};
use time::OffsetDateTime;
use time::format_description::well_known::Rfc3339;
use url::Url;

use super::resources::collect_report_from_root;
use crate::config::Target;
use crate::models::NodeInventory;
use crate::source::assembly::assemble;
use crate::source::reverse_dns::{
    DEFAULT_REVERSE_DNS_TIMEOUT, ReverseDnsResolver, resolve_hostname,
};
use crate::source::{InventorySource, RedfishReport};

type RedfishBmc = HttpBmc<DispatchingHttpClient<nv_redfish::bmc_http::reqwest::Client>>;

const MAX_REDIRECTS: usize = 3;
/// Identifies collector traffic in BMC access logs.
const USER_AGENT: &str = "fleet-intelligence-oob-collector";

pub struct RedfishSource {
    client: nv_redfish::bmc_http::reqwest::Client,
    dispatcher: Arc<Dispatcher>,
    reverse_dns: Option<ReverseDnsResolver>,
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
        let reverse_dns = match ReverseDnsResolver::new() {
            Ok(resolver) => Some(resolver),
            Err(error) => {
                tracing::warn!(%error, "reverse DNS is unavailable; BMC source host names will be omitted");
                None
            }
        };
        Ok(Self {
            client,
            dispatcher,
            reverse_dns,
        })
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
        let (result, host_name) = tokio::join!(
            self.collect_report(&target),
            resolve_hostname(
                self.reverse_dns.as_ref(),
                target.ip,
                DEFAULT_REVERSE_DNS_TIMEOUT,
            )
        );
        assemble(target, collected_at, host_name, result)
    }
}

fn now_rfc3339() -> String {
    OffsetDateTime::now_utc()
        .format(&Rfc3339)
        .unwrap_or_else(|_| "1970-01-01T00:00:00Z".to_string())
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
