// Copyright (c) 2025-2026, NVIDIA CORPORATION & AFFILIATES. All rights reserved.
//
// Licensed under the Apache License, Version 2.0 (the "License");
// you may not use this file except in compliance with the License.
// You may obtain a copy of the License at
//
//     http://www.apache.org/licenses/LICENSE-2.0
//
// Unless required by applicable law or agreed to in writing, software
// distributed under the License is distributed on an "AS IS" BASIS,
// WITHOUT WARRANTIES OR CONDITIONS OF ANY KIND, either express or implied.
// See the License for the specific language governing permissions and
// limitations under the License.
//
// SPDX-License-Identifier: Apache-2.0

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
    pub source: InventorySourceInfo,
    pub managers: Vec<ManagerInventory>,
    pub systems: Vec<SystemInventory>,
    pub chassis: Vec<ChassisInventory>,
    pub firmware: Vec<FirmwareInventory>,
    pub fabrics: Vec<FabricInventory>,
    pub domain_errors: Vec<DomainError>,
    pub target_error: Option<SanitizedErrorMessage>,
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
