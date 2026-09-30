//! Collector-to-BMC latency measured as TCP connect time.
//!
//! A connect completes when the BMC kernel answers the SYN, so the duration is
//! roughly one network round trip and excludes TLS and Redfish processing. The
//! stream is dropped immediately after connecting; no application bytes are
//! sent.

use std::net::{IpAddr, Ipv4Addr, SocketAddr};
use std::time::{Duration, Instant};

use bmc_error::SanitizedErrorMessage;
use tokio::net::TcpStream;

use super::now_rfc3339;
use crate::models::{LatencyMeasurement, LatencyMethod, LatencyStatus};

/// RFC 5737 TEST-NET-1 address. It never routes, so a successful connect means
/// something local (for example, a service mesh sidecar) terminated the TCP
/// connection.
const INTERCEPTION_CANARY: SocketAddr =
    SocketAddr::new(IpAddr::V4(Ipv4Addr::new(192, 0, 2, 1)), 443);
/// A local interceptor answers in well under a millisecond, and timing out is
/// the normal outcome, so the canary does not need the probe's longer timeout.
const INTERCEPTION_TIMEOUT: Duration = Duration::from_millis(500);

#[derive(Clone, Copy, Debug)]
pub struct TcpProbeConfig {
    pub attempts: u32,
    pub interval: Duration,
    pub timeout: Duration,
    pub min_successes: u32,
}

impl Default for TcpProbeConfig {
    fn default() -> Self {
        Self {
            attempts: 5,
            interval: Duration::from_millis(100),
            // Long enough to report far or congested paths as values rather
            // than failures, and to complete one Linux SYN retransmission
            // (~1 s); the round minimum discards that outlier.
            timeout: Duration::from_secs(2),
            min_successes: 3,
        }
    }
}

/// Whether and how the source measures latency for each target.
#[derive(Clone, Copy, Debug)]
pub enum LatencyProbe {
    Disabled,
    /// Interception was detected for this run; record it without probing.
    Intercepted,
    Enabled(TcpProbeConfig),
}

impl LatencyProbe {
    /// Resolve the probe mode for one collection run, checking once whether a
    /// local component is terminating outbound TCP.
    pub async fn for_run(enabled: bool, config: TcpProbeConfig) -> Self {
        if !enabled {
            return Self::Disabled;
        }
        if tcp_is_intercepted(INTERCEPTION_CANARY, INTERCEPTION_TIMEOUT).await {
            tracing::warn!(
                canary = %INTERCEPTION_CANARY,
                "outbound TCP is terminated locally; skipping BMC latency probes for this run"
            );
            return Self::Intercepted;
        }
        Self::Enabled(config)
    }
}

pub(crate) async fn tcp_is_intercepted(canary: SocketAddr, timeout: Duration) -> bool {
    matches!(
        tokio::time::timeout(timeout, TcpStream::connect(canary)).await,
        Ok(Ok(_))
    )
}

pub(crate) fn intercepted(destination: SocketAddr) -> LatencyMeasurement {
    not_measured(destination, LatencyStatus::Intercepted)
}

pub(crate) fn disabled(destination: SocketAddr) -> LatencyMeasurement {
    not_measured(destination, LatencyStatus::Disabled)
}

/// A round that sent no probe traffic, reported with the reason as its status.
fn not_measured(destination: SocketAddr, status: LatencyStatus) -> LatencyMeasurement {
    LatencyMeasurement {
        method: LatencyMethod::TcpConnect,
        status,
        destination: destination.to_string(),
        measured_at: now_rfc3339(),
        attempts: 0,
        successes: 0,
        min_us: None,
        median_us: None,
        error: None,
    }
}

pub(crate) fn unavailable(destination: SocketAddr, error: impl AsRef<str>) -> LatencyMeasurement {
    LatencyMeasurement {
        method: LatencyMethod::TcpConnect,
        status: LatencyStatus::Unavailable,
        destination: destination.to_string(),
        measured_at: now_rfc3339(),
        attempts: 0,
        successes: 0,
        min_us: None,
        median_us: None,
        error: Some(SanitizedErrorMessage::new(error)),
    }
}

/// Run one probe round against `destination`.
pub(crate) async fn probe(destination: SocketAddr, config: TcpProbeConfig) -> LatencyMeasurement {
    let measured_at = now_rfc3339();
    let mut durations = Vec::with_capacity(config.attempts as usize);
    let mut last_error = None;
    for attempt in 0..config.attempts {
        if attempt > 0 {
            tokio::time::sleep(config.interval).await;
        }
        let started = Instant::now();
        match tokio::time::timeout(config.timeout, TcpStream::connect(destination)).await {
            Ok(Ok(stream)) => {
                durations.push(started.elapsed());
                drop(stream);
            }
            Ok(Err(error)) => last_error = Some(error.to_string()),
            Err(_) => last_error = Some(format!("connect timed out after {:?}", config.timeout)),
        }
    }
    summarize(destination, measured_at, config, durations, last_error)
}

fn summarize(
    destination: SocketAddr,
    measured_at: String,
    config: TcpProbeConfig,
    mut durations: Vec<Duration>,
    last_error: Option<String>,
) -> LatencyMeasurement {
    durations.sort_unstable();
    let successes = durations.len() as u32;
    let ok = successes >= config.min_successes.max(1);
    let (min_us, median_us) = if ok {
        (
            durations.first().copied().map(micros),
            median(&durations).map(micros),
        )
    } else {
        (None, None)
    };
    let error = match (ok, last_error) {
        (false, Some(error)) => Some(SanitizedErrorMessage::new(format!(
            "{successes} of {} connects succeeded; last error: {error}",
            config.attempts
        ))),
        (false, None) => Some(SanitizedErrorMessage::new(format!(
            "{successes} of {} connects succeeded",
            config.attempts
        ))),
        (true, _) => None,
    };
    LatencyMeasurement {
        method: LatencyMethod::TcpConnect,
        status: if ok {
            LatencyStatus::Ok
        } else {
            LatencyStatus::Unavailable
        },
        destination: destination.to_string(),
        measured_at,
        attempts: config.attempts,
        successes,
        min_us,
        median_us,
        error,
    }
}

fn median(sorted: &[Duration]) -> Option<Duration> {
    let middle = sorted.len() / 2;
    match sorted.len() {
        0 => None,
        len if len % 2 == 1 => Some(sorted[middle]),
        _ => Some((sorted[middle - 1] + sorted[middle]) / 2),
    }
}

fn micros(duration: Duration) -> u64 {
    u64::try_from(duration.as_micros()).unwrap_or(u64::MAX)
}

#[cfg(test)]
mod tests {
    use tokio::net::TcpListener;

    use super::*;

    const DESTINATION: SocketAddr = SocketAddr::new(IpAddr::V4(Ipv4Addr::new(10, 0, 0, 1)), 443);

    fn ms(values: &[u64]) -> Vec<Duration> {
        values.iter().copied().map(Duration::from_millis).collect()
    }

    fn fast_config() -> TcpProbeConfig {
        TcpProbeConfig {
            attempts: 5,
            interval: Duration::ZERO,
            timeout: Duration::from_millis(500),
            min_successes: 3,
        }
    }

    #[test]
    fn summarize_reports_min_and_odd_median() {
        let measurement = summarize(
            DESTINATION,
            "t".into(),
            TcpProbeConfig::default(),
            ms(&[4, 1, 3, 2, 5]),
            None,
        );

        assert_eq!(measurement.status, LatencyStatus::Ok);
        assert_eq!(measurement.successes, 5);
        assert_eq!(measurement.min_us, Some(1_000));
        assert_eq!(measurement.median_us, Some(3_000));
        assert!(measurement.error.is_none());
    }

    #[test]
    fn summarize_averages_even_median() {
        let measurement = summarize(
            DESTINATION,
            "t".into(),
            TcpProbeConfig::default(),
            ms(&[4, 1, 3, 2]),
            Some("refused".into()),
        );

        assert_eq!(measurement.status, LatencyStatus::Ok);
        assert_eq!(measurement.median_us, Some(2_500));
        assert!(measurement.error.is_none());
    }

    #[test]
    fn summarize_withholds_latency_below_min_successes() {
        let measurement = summarize(
            DESTINATION,
            "t".into(),
            TcpProbeConfig::default(),
            ms(&[1, 2]),
            Some("connection refused".into()),
        );

        assert_eq!(measurement.status, LatencyStatus::Unavailable);
        assert_eq!(measurement.successes, 2);
        assert_eq!(measurement.min_us, None);
        assert_eq!(measurement.median_us, None);
        assert!(
            measurement
                .error
                .as_deref()
                .is_some_and(|error| error.contains("2 of 5") && error.contains("refused"))
        );
    }

    #[tokio::test]
    async fn probe_measures_listening_port() {
        let listener = TcpListener::bind("127.0.0.1:0").await.expect("bind");
        let destination = listener.local_addr().expect("addr");

        let measurement = probe(destination, fast_config()).await;

        assert_eq!(measurement.status, LatencyStatus::Ok);
        assert_eq!(measurement.successes, 5);
        assert_eq!(measurement.destination, destination.to_string());
        assert!(measurement.min_us <= measurement.median_us);
    }

    #[tokio::test]
    async fn probe_reports_refused_port_as_unavailable() {
        let listener = TcpListener::bind("127.0.0.1:0").await.expect("bind");
        let destination = listener.local_addr().expect("addr");
        drop(listener);

        let measurement = probe(destination, fast_config()).await;

        assert_eq!(measurement.status, LatencyStatus::Unavailable);
        assert_eq!(measurement.successes, 0);
        assert!(measurement.error.is_some());
    }

    #[tokio::test]
    async fn interception_detected_when_canary_accepts() {
        let listener = TcpListener::bind("127.0.0.1:0").await.expect("bind");
        let canary = listener.local_addr().expect("addr");

        assert!(tcp_is_intercepted(canary, Duration::from_millis(500)).await);
        drop(listener);
        assert!(!tcp_is_intercepted(canary, Duration::from_millis(500)).await);
    }
}
