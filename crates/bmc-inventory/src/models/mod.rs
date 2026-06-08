mod inventory;
mod node;

pub use inventory::{
    ChassisInventory, ChassisLocation, FirmwareInventory, InventorySourceInfo, ManagerInventory,
    PcieDeviceInventory, SystemInventory,
};
pub use node::{DomainError, InventoryUuid, NodeInventory};

pub const INVENTORY_SCHEMA_VERSION: &str = "inventory.v1alpha1";

#[cfg(test)]
mod tests;
