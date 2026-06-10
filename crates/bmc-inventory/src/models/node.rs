use std::fmt;

use serde::{Serialize, Serializer};
use uuid::Uuid;

use super::{
    ChassisInventory, FirmwareInventory, InventorySourceInfo, ManagerInventory, SystemInventory,
};

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct NodeInventory {
    pub schema_version: String,
    pub collected_at: String,
    pub uuid: Option<InventoryUuid>,
    pub source: InventorySourceInfo,
    pub primary_system_id: Option<String>,
    pub managers: Vec<ManagerInventory>,
    pub systems: Vec<SystemInventory>,
    pub chassis: Vec<ChassisInventory>,
    pub firmware: Vec<FirmwareInventory>,
    pub domain_errors: Vec<DomainError>,
    pub target_error: Option<String>,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DomainError {
    pub domain: String,
    pub resource: Option<String>,
    pub message: String,
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
