mod redfish;

use super::source::SourceRecord;
use super::types::NodeInventory;

pub use redfish::RedfishMapper;

pub trait InventoryMapper {
    fn map(&self, record: SourceRecord) -> NodeInventory;
}
