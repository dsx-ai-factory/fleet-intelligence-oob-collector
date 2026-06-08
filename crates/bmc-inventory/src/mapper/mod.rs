mod redfish;

use super::models::NodeInventory;
use super::source::SourceRecord;

pub use redfish::RedfishMapper;

pub trait InventoryMapper {
    fn map(&self, record: SourceRecord) -> NodeInventory;
}
