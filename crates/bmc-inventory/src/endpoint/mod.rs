// Copyright (c) 2026, NVIDIA CORPORATION & AFFILIATES. All rights reserved.
// SPDX-License-Identifier: Apache-2.0

//! BMC endpoint discovery, separate from credential resolution and collection.
//!
//! A source returns a complete listing or an error. Callers must not interpret
//! a discovery error as an empty listing or remove existing targets on failure.
//! Sources own their scope and authentication; no provider-specific API types
//! or BMC passwords belong in the shared endpoint model.

pub mod static_file;

use std::collections::BTreeMap;
use std::error::Error;
use std::net::IpAddr;
use std::sync::Arc;

use async_trait::async_trait;
use mac_address::MacAddress;

use crate::models::TargetKind;

/// Credential-free BMC connection and placement information. The BMC MAC remains
/// the Fleet Intelligence identity, regardless of where the endpoint came from.
#[derive(Clone, Debug)]
pub struct Endpoint {
    pub kind: TargetKind,
    pub name: Option<String>,
    pub compute_zone: Option<String>,
    pub node_group: Option<String>,
    pub ip: IpAddr,
    pub port: Option<u16>,
    pub mac: MacAddress,
    /// A reference into the separate credential store, never credential values.
    pub credentials_profile: Option<String>,
    /// Provider/user labels only. Derived labels are built during finalization.
    pub labels: BTreeMap<String, String>,
}

impl Endpoint {
    /// Address expected by the Redfish transport, with brackets around IPv6.
    pub fn address(&self) -> String {
        match (self.ip, self.port) {
            (IpAddr::V4(ip), Some(port)) => format!("{ip}:{port}"),
            (IpAddr::V6(ip), Some(port)) => format!("[{ip}]:{port}"),
            (IpAddr::V4(ip), None) => ip.to_string(),
            (IpAddr::V6(ip), None) => format!("[{ip}]"),
        }
    }
}

/// Discovery can fail without producing a usable target snapshot. Providers
/// must omit credentials, response bodies, and sensitive URLs from errors.
pub type EndpointSourceError = Box<dyn Error + Send + Sync>;

/// Shared immutable listing. Cloning a snapshot does not copy its endpoints.
pub type EndpointSnapshot = Arc<[Endpoint]>;

/// Provider-independent discovery boundary for static configuration and future
/// API integrations. Implementations must be safe to share between Tokio tasks.
/// Credential resolution, collection scheduling, and transport are separate.
#[async_trait]
pub trait EndpointSource: Send + Sync {
    async fn discover(&self) -> Result<EndpointSnapshot, EndpointSourceError>;
}

/// A single parsed static configuration snapshot. File reloads are owned by
/// the caller, so inventory and attestation share the same endpoint listing.
#[derive(Clone, Debug)]
pub struct StaticEndpointSource {
    endpoints: EndpointSnapshot,
}

impl StaticEndpointSource {
    pub fn new(endpoints: Vec<Endpoint>) -> Self {
        Self {
            endpoints: endpoints.into(),
        }
    }

    /// Borrow the configured definitions for synchronous configuration tools.
    pub fn endpoints(&self) -> &[Endpoint] {
        &self.endpoints
    }
}

#[async_trait]
impl EndpointSource for StaticEndpointSource {
    async fn discover(&self) -> Result<EndpointSnapshot, EndpointSourceError> {
        Ok(self.endpoints.clone())
    }
}
