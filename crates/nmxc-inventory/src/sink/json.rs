use crate::models::NvlinkDomainInventory;

pub fn render_json(inventories: &[NvlinkDomainInventory]) -> Result<String, serde_json::Error> {
    serde_json::to_string_pretty(inventories)
}
