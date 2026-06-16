mod backend;
mod json;
mod table;

use std::error::Error;

use super::models::NodeInventory;

pub use backend::{BackendClient, BackendHttpError, BackendSink, HttpBackendClient};
pub use json::JsonSink;
pub use table::TableSink;

pub trait InventorySink {
    fn write(&self, inventories: &[NodeInventory]) -> Result<(), Box<dyn Error>>;
}
