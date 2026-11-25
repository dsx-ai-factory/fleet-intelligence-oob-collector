// Copyright (c) 2026, NVIDIA CORPORATION & AFFILIATES. All rights reserved.
// SPDX-License-Identifier: Apache-2.0

//! BMC endpoint discovery, separate from credential resolution and collection.
//!
//! A source returns a complete listing or an error. Callers must not interpret
//! a discovery error as an empty listing or remove existing targets on failure.
//! Sources own their scope and authentication; no provider-specific API types
//! or BMC passwords belong in the shared endpoint model.

use std::collections::BTreeMap;
use std::error::Error;
use std::net::IpAddr;
use std::sync::Arc;

use async_trait::async_trait;
use mac_address::MacAddress;

use crate::models::TargetKind;

pub mod nico;

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

/// Longest name, placement, or label value accepted for a target.
pub(crate) const MAX_LABEL_VALUE_BYTES: usize = 1024;

/// Trim a name or placement value and enforce the shared value limits.
pub(crate) fn normalize_value(
    context: &str,
    value: Option<String>,
) -> Result<Option<String>, Box<dyn Error>> {
    let value = match value {
        Some(value) if value.trim().is_empty() => {
            return Err(format!("{context} cannot be blank").into());
        }
        Some(value) => Some(value.trim().to_string()),
        None => None,
    };
    if value
        .as_ref()
        .is_some_and(|value| value.len() > MAX_LABEL_VALUE_BYTES)
    {
        return Err(format!("{context} exceeds {MAX_LABEL_VALUE_BYTES} bytes").into());
    }
    Ok(value)
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

/// Write the credential-free source listing for discovery diagnostics. Pass
/// endpoints with effective placement applied (`SourceConfig::discover`); shared
/// labels and BMC credential resolution have not run yet.
pub fn write_discovery_json(
    endpoints: &[Endpoint],
    mut output: impl std::io::Write,
) -> Result<(), Box<dyn Error>> {
    let records: Vec<_> = endpoints
        .iter()
        .map(|endpoint| {
            serde_json::json!({
                "ip": endpoint.ip,
                "mac": endpoint.mac.to_string().to_uppercase(),
                "name": endpoint.name,
                "kind": endpoint.kind,
                "compute_zone": endpoint.compute_zone,
                "node_group": endpoint.node_group,
                "labels": endpoint.labels,
                "credentials_profile": endpoint.credentials_profile,
            })
        })
        .collect();
    serde_json::to_writer_pretty(&mut output, &records)?;
    output.write_all(b"\n")?;
    Ok(())
}

// Bound and sanitize provider identifiers before including them in diagnostics.
pub(crate) fn safe_source_reference(id: &str) -> String {
    id.chars()
        .take(128)
        .map(|c| {
            if c.is_ascii_alphanumeric() || "-_.:".contains(c) {
                c
            } else {
                '_'
            }
        })
        .collect()
}
