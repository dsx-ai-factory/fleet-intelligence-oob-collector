use std::fmt;

use bmc_error::{SanitizedErrorMessage, SanitizedUriReference};
use serde::{Deserialize, Serialize, Serializer};
use uuid::Uuid;

use super::{
    ChassisInventory, FabricInventory, FirmwareInventory, InventorySourceInfo, LatencyMeasurement,
    ManagerInventory, SystemInventory,
};

#[derive(Clone, Copy, Debug, Default, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum TargetKind {
    #[default]
    Unknown,
    Compute,
    #[serde(rename = "nvswitch")]
    NvSwitch,
    PowerShelf,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct NodeInventory {
    pub schema_version: String,
    pub collected_at: String,
    pub enrolled_at: Option<String>,
    pub kind: TargetKind,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub compute_zone: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub node_group: Option<String>,
    pub uuid: Option<InventoryUuid>,
    pub source: InventorySourceInfo,
    pub primary_system_id: Option<String>,
    pub managers: Vec<ManagerInventory>,
    pub systems: Vec<SystemInventory>,
    pub chassis: Vec<ChassisInventory>,
    pub firmware: Vec<FirmwareInventory>,
    pub fabrics: Vec<FabricInventory>,
    pub domain_errors: Vec<DomainError>,
    pub target_error: Option<SanitizedErrorMessage>,
    /// Collector-to-BMC network measurements. Not part of the inventory
    /// snapshot: the backend stores them separately from `oobInventory`.
    pub latency: Vec<LatencyMeasurement>,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DomainError {
    pub domain: String,
    pub resource: Option<SanitizedUriReference>,
    pub message: SanitizedErrorMessage,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct InventoryUuid(Uuid);

impl InventoryUuid {
    pub fn parse(value: &str) -> Option<Self> {
        Uuid::parse_str(value.trim()).ok().map(Self)
    }

    pub fn from_stable_name(name: &[u8]) -> Self {
        Self(Uuid::new_v5(&Uuid::NAMESPACE_URL, name))
    }
}

impl fmt::Display for InventoryUuid {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.0.fmt(f)
    }
}

impl Serialize for InventoryUuid {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        serializer.serialize_str(&self.to_string())
    }
}
