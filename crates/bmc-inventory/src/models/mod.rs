mod inventory;
mod network_latency;
mod node;

pub use inventory::{
    ChassisInventory, FirmwareInventory, InventoryCondition, InventoryLink, InventoryLinkReference,
    InventoryLocation, InventoryReference, InventorySourceInfo, InventoryStatus, ManagerInventory,
    MemorySummary, NvidiaGpuInventory, NvlinkTopologyInventory, PartLocation, PcieDeviceInventory,
    Placement, ProcessorInventory, ProcessorSummary, SystemInventory,
};
pub use network_latency::{
    NetworkLatencyObservation, NetworkLatencyProtocol, NetworkLatencyStatus,
};
pub use node::{DomainError, InventoryUuid, NodeInventory, TargetKind};

pub const INVENTORY_SCHEMA_VERSION: &str = "inventory.v1alpha1";

#[cfg(test)]
mod tests;
