mod nico;

use super::source::SourceRecord;
use super::types::NodeInventory;

pub use nico::NicoMapper;

pub trait InventoryMapper {
    fn map(&self, record: SourceRecord) -> NodeInventory;
}
