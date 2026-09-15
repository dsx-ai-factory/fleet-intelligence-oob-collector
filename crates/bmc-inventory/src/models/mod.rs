mod inventory;
mod node;

pub use inventory::{
    ChassisInventory, ChassisLocation, FirmwareInventory, InventoryCondition, InventoryLink,
    InventoryLinkReference, InventoryReference, InventorySourceInfo, InventoryStatus,
    ManagerInventory, MemorySummary, PartLocation, PcieDeviceInventory, Placement,
    ProcessorInventory, ProcessorSummary, SystemInventory,
};
pub use node::{DomainError, InventoryUuid, NodeInventory, TargetKind};

pub const INVENTORY_SCHEMA_VERSION: &str = "inventory.v1alpha1";

#[cfg(test)]
mod tests;
