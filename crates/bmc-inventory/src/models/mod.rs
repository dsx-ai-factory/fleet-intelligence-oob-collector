mod node;
mod resources;

pub use node::{DomainError, InventoryUuid, NodeInventory, TargetKind};
pub use resources::{
    ChassisInventory, FabricInventory, FirmwareInventory, InventoryCondition, InventoryLink,
    InventoryLinkReference, InventoryLocation, InventoryReference, InventorySourceInfo,
    InventoryStatus, ManagerInventory, MemorySummary, NvidiaGpuInventory, NvidiaSwitchInventory,
    NvlinkTopologyInventory, PartLocation, PcieDeviceInventory, Placement, ProcessorInventory,
    ProcessorSummary, SwitchInventory, SystemInventory,
};

pub const INVENTORY_SCHEMA_VERSION: &str = "inventory.v1alpha1";
