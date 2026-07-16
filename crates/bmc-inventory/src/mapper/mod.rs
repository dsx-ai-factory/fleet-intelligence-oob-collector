mod redfish;
mod reverse_dns;

use async_trait::async_trait;

use super::models::NodeInventory;
use super::source::SourceRecord;

pub use redfish::RedfishMapper;

#[async_trait]
pub trait InventoryMapper {
    async fn map(&self, record: SourceRecord) -> NodeInventory;
}
