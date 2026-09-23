mod node;
mod resources;

pub use node::{DomainError, InventoryUuid, NodeInventory, TargetKind};
pub use resources::{
    ChassisInventory, FirmwareInventory, InventoryCondition, InventoryLink, InventoryLinkReference,
    InventoryLocation, InventoryReference, InventorySourceInfo, InventoryStatus, ManagerInventory,
    MemorySummary, NvidiaGpuInventory, NvlinkTopologyInventory, PartLocation, PcieDeviceInventory,
    Placement, ProcessorInventory, ProcessorSummary, SystemInventory,
};

pub const INVENTORY_SCHEMA_VERSION: &str = "inventory.v1alpha1";

#[cfg(test)]
mod serialization_tests;
