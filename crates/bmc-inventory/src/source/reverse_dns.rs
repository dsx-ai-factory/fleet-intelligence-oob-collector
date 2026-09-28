use std::net::IpAddr;
use std::time::Duration;

use async_trait::async_trait;
use hickory_resolver::TokioResolver;
use hickory_resolver::proto::rr::RData;

pub(super) const DEFAULT_REVERSE_DNS_TIMEOUT: Duration = Duration::from_secs(2);

#[async_trait]
pub(super) trait HostnameResolver: Send + Sync {
    async fn reverse_lookup(&self, ip: IpAddr) -> Result<Option<String>, String>;
}

pub(super) struct ReverseDnsResolver {
    resolver: TokioResolver,
}

impl ReverseDnsResolver {
    pub(super) fn new() -> Result<Self, String> {
        let resolver = TokioResolver::builder_tokio()
            .map_err(|error| error.to_string())?
            .build()
            .map_err(|error| error.to_string())?;
        Ok(Self { resolver })
    }
}

#[async_trait]
impl HostnameResolver for ReverseDnsResolver {
    async fn reverse_lookup(&self, ip: IpAddr) -> Result<Option<String>, String> {
        let lookup = self
            .resolver
            .reverse_lookup(ip)
            .await
            .map_err(|error| error.to_string())?;

        Ok(lookup
            .answers()
            .iter()
            .find_map(|record| match &record.data {
                RData::PTR(name) => normalize_hostname(&name.to_string()),
                _ => None,
            }))
    }
}

pub(super) async fn resolve_hostname<R>(
    resolver: Option<&R>,
    ip: IpAddr,
    timeout: Duration,
) -> Option<String>
where
    R: HostnameResolver + ?Sized,
{
    let resolver = resolver?;
    match tokio::time::timeout(timeout, resolver.reverse_lookup(ip)).await {
        Ok(Ok(host_name)) => host_name,
        Ok(Err(error)) => {
            tracing::debug!(%ip, %error, "BMC reverse DNS lookup failed");
            None
        }
        Err(_) => {
            tracing::debug!(
                %ip,
                timeout_seconds = timeout.as_secs_f64(),
                "BMC reverse DNS lookup timed out"
            );
            None
        }
    }
}

fn normalize_hostname(host_name: &str) -> Option<String> {
    let host_name = host_name.trim().trim_end_matches('.');
    (!host_name.is_empty()).then(|| host_name.to_ascii_lowercase())
}

#[cfg(test)]
mod tests {
    use super::*;

    struct StubResolver {
        result: Result<Option<String>, String>,
        delay: Duration,
    }

    #[async_trait]
    impl HostnameResolver for StubResolver {
        async fn reverse_lookup(&self, _ip: IpAddr) -> Result<Option<String>, String> {
            tokio::time::sleep(self.delay).await;
            self.result.clone()
        }
    }

    fn ip() -> IpAddr {
        "192.0.2.10".parse().expect("IP address")
    }

    #[tokio::test]
    async fn returns_resolved_hostname() {
        let resolver = StubResolver {
            result: Ok(Some("bmc-a.example.com".to_string())),
            delay: Duration::ZERO,
        };

        assert_eq!(
            resolve_hostname(Some(&resolver), ip(), Duration::from_secs(1)).await,
            Some("bmc-a.example.com".to_string())
        );
    }

    #[tokio::test]
    async fn lookup_failure_is_non_fatal() {
        let resolver = StubResolver {
            result: Err("no PTR record".to_string()),
            delay: Duration::ZERO,
        };

        assert_eq!(
            resolve_hostname(Some(&resolver), ip(), Duration::from_secs(1)).await,
            None
        );
    }

    #[tokio::test]
    async fn lookup_timeout_is_non_fatal() {
        let resolver = StubResolver {
            result: Ok(Some("too-late.example.com".to_string())),
            delay: Duration::from_millis(50),
        };

        assert_eq!(
            resolve_hostname(Some(&resolver), ip(), Duration::from_millis(1)).await,
            None
        );
    }

    #[test]
    fn normalizes_dns_name() {
        assert_eq!(
            normalize_hostname(" BMC-A.Example.COM. "),
            Some("bmc-a.example.com".to_string())
        );
        assert_eq!(normalize_hostname("."), None);
    }
}
