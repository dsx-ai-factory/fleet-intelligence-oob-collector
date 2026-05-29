pub mod config;
pub mod inventory;

use std::error::Error;

use inventory::mapper::{InventoryMapper, RedfishMapper};
use inventory::sink::{InventorySink, TableSink};
use inventory::source::{InventorySource, RedfishSource};
use mac_address::MacAddress;

pub struct InventoryOptions {
    pub config: String,
    pub target: Option<String>,
    pub port: u16,
    pub boot_mac: Option<MacAddress>,
    pub insecure_tls: bool,
    pub detail: bool,
}

pub async fn run_inventory(options: InventoryOptions) -> Result<(), Box<dyn Error>> {
    let mut targets = config::load_targets(&options.config)?;
    if let Some(target_name) = &options.target {
        targets.retain(|target| target.name == *target_name);
        if targets.is_empty() {
            return Err(format!("target {target_name:?} not found").into());
        }
    }

    let source = RedfishSource::new(options.port, options.insecure_tls, options.boot_mac);
    let mapper = RedfishMapper;
    let sink = TableSink::new(options.detail);

    let mut inventories = Vec::with_capacity(targets.len());
    for target in targets {
        let source_record = source.collect(target).await;
        inventories.push(mapper.map(source_record));
    }

    sink.write(&inventories)?;
    Ok(())
}
