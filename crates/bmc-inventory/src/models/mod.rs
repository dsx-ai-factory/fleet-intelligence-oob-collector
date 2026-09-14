mod inventory;
mod node;

pub use inventory::{
    ChassisInventory, ChassisLocation, FirmwareInventory, InventoryCondition, InventoryReference,
    InventorySourceInfo, InventoryStatus, ManagerInventory, PcieDeviceInventory,
    ProcessorInventory, SystemInventory,
};
pub use node::{DomainError, InventoryUuid, NodeInventory, TargetKind};

pub const INVENTORY_SCHEMA_VERSION: &str = "inventory.v1alpha1";

#[cfg(test)]
mod tests;
