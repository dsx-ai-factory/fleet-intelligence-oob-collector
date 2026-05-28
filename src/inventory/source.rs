use std::error::Error;
use std::sync::Arc;
use std::time::Duration;

use bmc_explorer::{ErrorClass, nv_generate_exploration_report};
use mac_address::MacAddress;
use model::site_explorer::EndpointExplorationReport;
use nv_redfish::bmc_http::{BmcCredentials, CacheSettings, HttpBmc};
use nv_redfish::{Bmc, ServiceRoot};
use url::Url;

use crate::config::Target;

type RedfishBmc = HttpBmc<nv_redfish::bmc_http::reqwest::Client>;

pub struct RedfishSource {
    port: u16,
    insecure_tls: bool,
    boot_mac: Option<MacAddress>,
}

pub struct RedfishRecord {
    pub target: Target,
    pub result: Result<EndpointExplorationReport, String>,
}

impl RedfishSource {
    pub fn new(port: u16, insecure_tls: bool, boot_mac: Option<MacAddress>) -> Self {
        Self {
            port,
            insecure_tls,
            boot_mac,
        }
    }

    pub async fn collect(&self, target: Target) -> RedfishRecord {
        let result = self.collect_report(&target).await;
        RedfishRecord { target, result }
    }

    async fn collect_report(&self, target: &Target) -> Result<EndpointExplorationReport, String> {
        let root = service_root(target, self.port, self.insecure_tls)
            .await
            .map_err(|err| err.to_string())?;
        let config = bmc_explorer::Config {
            boot_interface_mac: self.boot_mac,
            error_classifier: &error_classifier,
            retry_timeout: Duration::from_secs(2),
        };
        nv_generate_exploration_report(root, &config)
            .await
            .map_err(|err| err.to_string())
    }
}

async fn service_root(
    target: &Target,
    default_port: u16,
    insecure_tls: bool,
) -> Result<Arc<ServiceRoot<RedfishBmc>>, Box<dyn Error>> {
    let url = bmc_url(&target.address, default_port)?;
    let client = nv_redfish::bmc_http::reqwest::Client::with_params(
        nv_redfish::bmc_http::reqwest::ClientParams::new()
            .timeout(Duration::from_secs(10))
            .connect_timeout(Duration::from_secs(5))
            .accept_invalid_certs(insecure_tls)
            .max_redirects(0),
    )?;
    let bmc = Arc::new(HttpBmc::new(
        client,
        url,
        BmcCredentials::new(target.username.clone(), target.password.clone()),
        CacheSettings::with_capacity(32),
    ));
    Ok(ServiceRoot::new(bmc).await?.into())
}

fn bmc_url(address: &str, default_port: u16) -> Result<Url, Box<dyn Error>> {
    if address.starts_with("http://") || address.starts_with("https://") {
        return Ok(address.parse()?);
    }
    Ok(format!("https://{address}:{default_port}").parse()?)
}

fn error_classifier(err: &<RedfishBmc as Bmc>::Error) -> Option<ErrorClass> {
    match err {
        nv_redfish::bmc_http::reqwest::BmcError::InvalidResponse { status, .. } => {
            match status.as_u16() {
                404 => Some(ErrorClass::NotFound),
                500 => Some(ErrorClass::InternalServerError),
                _ => None,
            }
        }
        _ => None,
    }
}
