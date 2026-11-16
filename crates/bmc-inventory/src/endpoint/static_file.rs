// Copyright (c) 2026, NVIDIA CORPORATION & AFFILIATES. All rights reserved.
// SPDX-License-Identifier: Apache-2.0

//! Static JSON endpoint discovery. Shared settings and passwords stay outside it.

use std::collections::BTreeMap;
use std::net::IpAddr;
use std::path::PathBuf;

use async_trait::async_trait;
use mac_address::MacAddress;
use serde::Deserialize;

use super::{Endpoint, EndpointSnapshot, EndpointSource, EndpointSourceError};
use crate::models::TargetKind;

/// A JSON array of endpoint definitions, reread on every discovery.
#[derive(Clone, Debug)]
pub struct StaticFileEndpointSource {
    file: PathBuf,
}

impl StaticFileEndpointSource {
    /// Paths must be absolute; config loaders root them beside the TOML file.
    pub fn new(file: PathBuf) -> Result<Self, EndpointSourceError> {
        if !file.is_absolute() {
            return Err("static endpoint source requires an absolute file path".into());
        }
        Ok(Self { file })
    }

    pub(crate) fn read_sync(&self) -> Result<Vec<Endpoint>, EndpointSourceError> {
        let contents =
            std::fs::read(&self.file).map_err(|_| "cannot read static endpoint_source.file")?;
        parse_endpoints(&contents)
    }
}

#[async_trait]
impl EndpointSource for StaticFileEndpointSource {
    async fn discover(&self) -> Result<EndpointSnapshot, EndpointSourceError> {
        let contents = tokio::fs::read(&self.file)
            .await
            .map_err(|_| "cannot read static endpoint_source.file")?;
        Ok(parse_endpoints(&contents)?.into())
    }
}

fn parse_endpoints(contents: &[u8]) -> Result<Vec<Endpoint>, EndpointSourceError> {
    let targets: Vec<RawTargetConfig> = serde_json::from_slice(contents).map_err(|error| {
        format!("invalid static targets JSON at line {}, column {}; expected an array of endpoint records", error.line(), error.column())
    })?;
    if targets.is_empty() {
        return Err("no BMC targets found in static endpoint_source.file".into());
    }
    targets
        .into_iter()
        .enumerate()
        .map(|(index, target)| target.into_endpoint(&format!("targets[{index}]")))
        .collect()
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct RawTargetConfig {
    #[serde(default)]
    kind: Option<TargetKind>,
    #[serde(default)]
    name: Option<String>,
    #[serde(default)]
    compute_zone: Option<String>,
    #[serde(default)]
    node_group: Option<String>,
    ip: IpAddr,
    #[serde(default)]
    port: Option<u16>,
    #[serde(default)]
    mac: Option<String>,
    #[serde(default)]
    credentials_profile: Option<String>,
    #[serde(default)]
    labels: BTreeMap<String, String>,
}

impl RawTargetConfig {
    pub(crate) fn into_endpoint(self, context: &str) -> Result<Endpoint, EndpointSourceError> {
        let mac = self
            .mac
            .as_deref()
            .ok_or_else(|| format!("{context}.mac is required"))?;
        if mac.trim().is_empty() {
            return Err(format!("{context}.mac cannot be blank").into());
        }
        let mac = mac
            .trim()
            .parse::<MacAddress>()
            .map_err(|error| format!("{context}.mac is invalid: {error}"))?;
        Ok(Endpoint {
            kind: self.kind.unwrap_or_default(),
            name: self.name,
            compute_zone: self.compute_zone,
            node_group: self.node_group,
            ip: self.ip,
            port: self.port,
            mac,
            credentials_profile: self.credentials_profile,
            labels: self.labels,
        })
    }
}
