// Copyright (c) 2026, NVIDIA CORPORATION & AFFILIATES. All rights reserved.
// SPDX-License-Identifier: Apache-2.0

//! Authenticated NICo BMC proxy transport shared by inventory and attestation.

use std::error::Error;
use std::path::{Path, PathBuf};
use std::time::Duration;

use serde::Deserialize;
use url::Url;

#[derive(Clone, Debug, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct BmcProxyConfig {
    pub url: String,
    pub ca_cert_path: PathBuf,
    pub client_cert_path: PathBuf,
    pub client_key_path: PathBuf,
}

impl BmcProxyConfig {
    pub fn endpoint(&self) -> Result<Url, Box<dyn Error>> {
        let url = Url::parse(&self.url).map_err(|_| "invalid bmc_proxy.url")?;
        if url.scheme() != "https"
            || url.host_str().is_none()
            || !url.username().is_empty()
            || url.password().is_some()
            || url.query().is_some()
            || url.fragment().is_some()
            || url.path() != "/"
        {
            return Err("bmc_proxy.url must be an HTTPS origin without credentials, path, query, or fragment".into());
        }
        Ok(url)
    }

    /// Resolve paths without canonicalizing symlinks used for Secret rotation.
    pub fn resolve_paths(&mut self, directory: &Path) -> Result<(), Box<dyn Error>> {
        self.endpoint()?;
        for path in [
            &mut self.ca_cert_path,
            &mut self.client_cert_path,
            &mut self.client_key_path,
        ] {
            if path.as_os_str().is_empty() {
                return Err("bmc_proxy TLS file paths must not be empty".into());
            }
            *path = std::path::absolute(directory.join(&*path))?;
        }
        Ok(())
    }

    /// Always verify the proxy, regardless of the direct-BMC insecure TLS flag.
    pub fn client(&self, timeout: Duration) -> Result<reqwest::Client, Box<dyn Error>> {
        self.endpoint()?;
        let ca =
            std::fs::read(&self.ca_cert_path).map_err(|_| "failed to read bmc_proxy CA bundle")?;
        let certificates = reqwest::Certificate::from_pem_bundle(&ca)
            .map_err(|_| "invalid bmc_proxy CA bundle")?;
        if certificates.is_empty() {
            return Err("bmc_proxy CA bundle contains no certificates".into());
        }
        let mut identity = std::fs::read(&self.client_cert_path)
            .map_err(|_| "failed to read bmc_proxy client certificate")?;
        identity.push(b'\n');
        identity.extend(
            std::fs::read(&self.client_key_path)
                .map_err(|_| "failed to read bmc_proxy client key")?,
        );
        let identity = reqwest::Identity::from_pem(&identity)
            .map_err(|_| "invalid bmc_proxy client certificate or key")?;
        let mut builder = reqwest::Client::builder()
            .https_only(true)
            .no_proxy()
            .tls_built_in_root_certs(false)
            .identity(identity)
            .timeout(timeout)
            .connect_timeout(timeout.min(Duration::from_secs(10)))
            .redirect(reqwest::redirect::Policy::none())
            .user_agent("fleet-intelligence-oob-collector");
        for certificate in certificates {
            builder = builder.add_root_certificate(certificate);
        }
        builder
            .build()
            .map_err(|_| "failed to build bmc_proxy TLS client".into())
    }
}

/// MAC routing preserves the configured Fleet identity even if the BMC IP changes.
pub fn forwarded_header(mac: &str) -> Result<http::HeaderValue, Box<dyn Error>> {
    if mac.len() != 17
        || !mac.bytes().enumerate().all(|(i, byte)| {
            if i % 3 == 2 {
                byte == b':'
            } else {
                byte.is_ascii_hexdigit()
            }
        })
    {
        return Err("invalid BMC MAC for proxy routing".into());
    }
    Ok(format!("mac=\"{mac}\"").parse()?)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn config(url: &str) -> BmcProxyConfig {
        BmcProxyConfig {
            url: url.into(),
            ca_cert_path: "ca.pem".into(),
            client_cert_path: "client.pem".into(),
            client_key_path: "client.key".into(),
        }
    }

    #[test]
    fn proxy_requires_an_https_origin_and_nonempty_tls_paths() {
        for url in [
            "http://proxy.example.com",
            "https://user:secret@proxy.example.com",
            "https://proxy.example.com/path",
            "https://proxy.example.com?token=secret",
            "https://proxy.example.com#secret",
            "not a URL",
        ] {
            let error = config(url).endpoint().unwrap_err().to_string();
            assert!(!error.contains("secret"));
        }
        let mut valid = config("https://proxy.example.com:1079");
        valid.resolve_paths(Path::new("/tmp/config")).unwrap();
        assert_eq!(valid.client_key_path, Path::new("/tmp/config/client.key"));
        valid.client_key_path = "".into();
        assert!(valid.resolve_paths(Path::new("/tmp/config")).is_err());
    }
}
