mod redfish;

use async_trait::async_trait;
use model::site_explorer::EndpointExplorationReport;

use crate::config::Target;

pub use redfish::RedfishSource;

pub struct SourceRecord {
    pub target: Target,
    pub result: Result<EndpointExplorationReport, String>,
}

#[async_trait]
pub trait InventorySource {
    async fn collect(&self, target: Target) -> SourceRecord;
}
