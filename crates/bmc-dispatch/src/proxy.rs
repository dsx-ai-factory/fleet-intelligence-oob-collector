// Copyright (c) 2026, NVIDIA CORPORATION & AFFILIATES. All rights reserved.
// SPDX-License-Identifier: Apache-2.0

//! NICo reverse-proxy routing and workload TLS identity, shared by both collectors.

use std::error::Error;
use std::path::{Path, PathBuf};
use std::time::Duration;

use http::{HeaderMap, HeaderValue, header};
use serde::Deserialize;
use url::Url;

/// The collector's identity for NICo's BMC proxy. Contains no BMC credentials.
#[derive(Clone, Debug, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct NicoBmcProxyConfig {
    pub url: String,
    pub ca_file: PathBuf,
    pub client_cert_file: PathBuf,
    pub client_key_file: PathBuf,
}

impl NicoBmcProxyConfig {
    /// Validate the proxy origin and resolve identity paths beside the TOML file.
    pub fn resolve_paths(&mut self, directory: &Path) -> Result<(), Box<dyn Error>> {
        self.origin()?;
        for path in [
            &mut self.ca_file,
            &mut self.client_cert_file,
            &mut self.client_key_file,
        ] {
            if path.as_os_str().is_empty() {
                return Err("NICo BMC proxy identity paths must not be empty".into());
            }
            *path = std::path::absolute(directory.join(&*path))?;
        }
        Ok(())
    }

    fn origin(&self) -> Result<Url, Box<dyn Error>> {
        let url = Url::parse(&self.url).map_err(|_| "invalid NICo BMC proxy URL")?;
        if url.scheme() != "https"
            || url.host_str().is_none()
            || !url.username().is_empty()
            || url.password().is_some()
            || !matches!(url.path(), "" | "/")
            || url.query().is_some()
            || url.fragment().is_some()
        {
            return Err("NICo BMC proxy URL must be an HTTPS origin without credentials, path, query, or fragment".into());
        }
        Ok(url)
    }

    /// Load the current workload identity for one collection attempt. Failed
    /// reloads fail closed; each new target attempt reloads rotated Secret files.
    pub async fn connect(
        &self,
        target: &Url,
        timeout: Duration,
        user_agent: &str,
    ) -> Result<(reqwest::Client, ProxyRoute), Box<dyn Error>> {
        let route = ProxyRoute::new(self.origin()?, target.clone())?;
        let ca = tokio::fs::read(&self.ca_file)
            .await
            .map_err(|_| "cannot read NICo BMC proxy CA file")?;
        let mut identity = tokio::fs::read(&self.client_cert_file)
            .await
            .map_err(|_| "cannot read NICo BMC proxy client certificate")?;
        identity.push(b'\n');
        identity.extend(
            tokio::fs::read(&self.client_key_file)
                .await
                .map_err(|_| "cannot read NICo BMC proxy client key")?,
        );
        let identity = reqwest::Identity::from_pem(&identity)
            .map_err(|_| "invalid NICo BMC proxy client identity")?;
        let certificates = reqwest::Certificate::from_pem_bundle(&ca)
            .map_err(|_| "invalid NICo BMC proxy CA bundle")?;
        if certificates.is_empty() {
            return Err("NICo BMC proxy CA bundle contains no certificates".into());
        }
        let mut builder = reqwest::Client::builder()
            .user_agent(user_agent)
            .https_only(true)
            .tls_built_in_root_certs(false)
            .identity(identity)
            .timeout(timeout)
            .connect_timeout(timeout.min(Duration::from_secs(10)))
            // The proxy handles safe BMC redirects. Never redirect the client
            // identity or target-routing header to another server.
            .redirect(reqwest::redirect::Policy::none());
        for certificate in certificates {
            builder = builder.add_root_certificate(certificate);
        }
        let client = builder
            .build()
            .map_err(|_| "cannot initialize NICo BMC proxy TLS client")?;
        Ok((client, route))
    }
}

/// A fixed proxy connection for one actual BMC, including links advertised by it.
#[derive(Clone, Debug)]
pub struct ProxyRoute {
    proxy: Url,
    target: Url,
    forwarded: HeaderValue,
}

impl ProxyRoute {
    fn new(proxy: Url, target: Url) -> Result<Self, Box<dyn Error>> {
        let host = target.host_str().ok_or("BMC target has no host")?;
        if target.scheme() != "https" || target.port_or_known_default() != Some(443) {
            return Err("NICo BMC proxy targets must use Redfish HTTPS port 443".into());
        }
        let forwarded = HeaderValue::from_str(&format!("host=\"{host}\""))
            .map_err(|_| "invalid NICo BMC proxy target")?;
        Ok(Self {
            proxy,
            target,
            forwarded,
        })
    }

    /// Preserve Redfish path/query while forcing every exchange to the proxy.
    pub fn route(&self, url: &Url) -> Result<Url, &'static str> {
        if !url.username().is_empty()
            || url.password().is_some()
            || (url.origin() != self.target.origin() && url.origin() != self.proxy.origin())
            || !(url.path() == "/redfish/v1" || url.path().starts_with("/redfish/v1/"))
        {
            return Err("Redfish reference is outside the configured NICo proxy target");
        }
        let mut routed = self.proxy.clone();
        routed.set_path(url.path());
        routed.set_query(url.query());
        Ok(routed)
    }

    /// Set the target independently of resource headers and strip BMC secrets.
    pub fn headers(&self, headers: &HeaderMap) -> HeaderMap {
        let mut headers = headers.clone();
        headers.remove(header::AUTHORIZATION);
        headers.remove(header::PROXY_AUTHORIZATION);
        headers.remove("x-auth-token");
        headers.insert("forwarded", self.forwarded.clone());
        headers
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn route() -> ProxyRoute {
        ProxyRoute::new(
            "https://proxy.example.com:1079".parse().unwrap(),
            "https://192.0.2.1".parse().unwrap(),
        )
        .unwrap()
    }

    #[test]
    fn pins_origin_preserves_redfish_reference_and_strips_device_credentials() {
        let route = route();
        let url = route
            .route(
                &"https://192.0.2.1/redfish/v1/Systems/a%20b?$expand=*"
                    .parse()
                    .unwrap(),
            )
            .unwrap();
        assert_eq!(
            url.as_str(),
            "https://proxy.example.com:1079/redfish/v1/Systems/a%20b?$expand=*"
        );
        assert_eq!(route.route(&url).unwrap(), url);
        let mut headers = HeaderMap::new();
        for name in [
            "authorization",
            "proxy-authorization",
            "x-auth-token",
            "forwarded",
        ] {
            headers.insert(name, HeaderValue::from_static("must-not-be-forwarded"));
        }
        headers.insert("accept", HeaderValue::from_static("application/json"));
        let headers = route.headers(&headers);
        for name in ["authorization", "proxy-authorization", "x-auth-token"] {
            assert!(!headers.contains_key(name));
        }
        assert_eq!(headers["forwarded"], "host=\"192.0.2.1\"");
        assert_eq!(headers["accept"], "application/json");
    }

    #[test]
    fn rejects_references_outside_fixed_target() {
        for url in [
            "https://192.0.2.2/redfish/v1/Systems",
            "http://192.0.2.1/redfish/v1/Systems",
            "https://192.0.2.1:8443/redfish/v1/Systems",
            "https://user:secret@192.0.2.1/redfish/v1/Systems",
            "https://192.0.2.1/redfish/v10/Systems",
            "https://192.0.2.1/not-redfish",
        ] {
            assert!(route().route(&url.parse().unwrap()).is_err(), "{url}");
        }
    }

    #[test]
    fn validates_proxy_origin_and_roots_identity_paths() {
        let mut config = NicoBmcProxyConfig {
            url: "https://proxy.example.com:1079".into(),
            ca_file: "ca.crt".into(),
            client_cert_file: "tls.crt".into(),
            client_key_file: "tls.key".into(),
        };
        config.resolve_paths(Path::new("/tmp/config")).unwrap();
        assert_eq!(config.client_key_file, Path::new("/tmp/config/tls.key"));
        for url in [
            "http://proxy.example.com",
            "https://user:secret@proxy.example.com",
            "https://proxy.example.com/path",
            "https://proxy.example.com?key=secret",
            "https://proxy.example.com#fragment",
        ] {
            config.url = url.into();
            let error = config
                .resolve_paths(Path::new("/tmp"))
                .unwrap_err()
                .to_string();
            assert!(!error.contains("secret"));
        }
    }
}
