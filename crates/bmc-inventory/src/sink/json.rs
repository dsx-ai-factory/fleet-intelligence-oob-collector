use std::error::Error;

use crate::models::NodeInventory;

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
    use crate::models::{INVENTORY_SCHEMA_VERSION, InventorySourceInfo, NodeInventory};

    #[test]
    fn json_sink_renders_backend_payload_shape() {
        let rendered = render_json(&[inventory()]).expect("json");

        assert!(rendered.contains(r#""sourceType": "redfish""#));
        assert!(!rendered.contains(r#""target""#));
        assert!(rendered.contains(r#""systems": []"#));
        assert!(rendered.contains(r#""targetError": null"#));
    }

    fn inventory() -> NodeInventory {
        NodeInventory {
            schema_version: INVENTORY_SCHEMA_VERSION.to_string(),
            collected_at: "2026-01-01T00:00:00Z".to_string(),
            enrolled_at: None,
            kind: crate::models::TargetKind::Unknown,
            name: None,
            compute_zone: None,
            node_group: None,
            uuid: None,
            source: InventorySourceInfo {
                source_type: "redfish".to_string(),
                mac: "00:11:22:33:44:55".to_string(),
                address: "10.0.0.1".to_string(),
                host_name: None,
                redfish_version: None,
                service_uuid: None,
                vendor: None,
            },
            primary_system_id: None,
            managers: Vec::new(),
            systems: Vec::new(),
            chassis: Vec::new(),
            firmware: Vec::new(),
            fabrics: Vec::new(),
            domain_errors: Vec::new(),
            target_error: None,
        }
    }
}
