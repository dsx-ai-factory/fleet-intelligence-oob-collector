use std::error::Error;

use crate::inventory::types::NodeInventory;

use super::InventorySink;

#[derive(Default)]
pub struct JsonSink;

impl InventorySink for JsonSink {
    fn write(&self, inventories: &[NodeInventory]) -> Result<(), Box<dyn Error>> {
        println!("{}", render_json(inventories)?);
        Ok(())
    }
}

fn render_json(inventories: &[NodeInventory]) -> Result<String, serde_json::Error> {
    serde_json::to_string_pretty(inventories)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::inventory::types::{INVENTORY_SCHEMA_VERSION, InventorySourceInfo, NodeInventory};

    #[test]
    fn json_sink_renders_backend_payload_shape() {
        let rendered = render_json(&[inventory("node-a")]).expect("json");

        assert!(rendered.contains(r#""sourceType": "redfish""#));
        assert!(rendered.contains(r#""target": "node-a""#));
        assert!(rendered.contains(r#""node": null"#));
        assert!(rendered.contains(r#""targetError": null"#));
    }

    fn inventory(target: &str) -> NodeInventory {
        NodeInventory {
            schema_version: INVENTORY_SCHEMA_VERSION.to_string(),
            collected_at: "2026-01-01T00:00:00Z".to_string(),
            node: None,
            source: InventorySourceInfo {
                source_type: "redfish".to_string(),
                target: target.to_string(),
                address: "10.0.0.1".to_string(),
                redfish_version: None,
                service_uuid: None,
                vendor: None,
            },
            manager: None,
            chassis: Vec::new(),
            devices: Vec::new(),
            firmware: Vec::new(),
            domain_errors: Vec::new(),
            target_error: None,
        }
    }
}
