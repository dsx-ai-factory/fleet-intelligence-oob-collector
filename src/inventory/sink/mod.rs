mod table;

use std::error::Error;

use super::types::NodeInventory;

pub use table::TableSink;

pub trait InventorySink {
    fn write(&self, inventories: &[NodeInventory]) -> Result<(), Box<dyn Error>>;
}
