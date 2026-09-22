use std::net::IpAddr;
use std::thread;
use std::time::Duration;

use async_trait::async_trait;
use time::OffsetDateTime;
use time::format_description::well_known::Rfc3339;

use crate::models::{NetworkLatencyObservation, NetworkLatencyProtocol, NetworkLatencyStatus};

const DEFAULT_PROBE_COUNT: u16 = 5;
const DEFAULT_PROBE_TIMEOUT: Duration = Duration::from_millis(500);
const DEFAULT_PROBE_INTERVAL: Duration = Duration::from_millis(100);

#[async_trait]
pub(crate) trait IcmpProbe: Send + Sync {
    async fn observe(&self, address: IpAddr) -> NetworkLatencyObservation;
}

pub(crate) struct SystemIcmpProbe {
    probe_count: u16,
    timeout: Duration,
    interval: Duration,
}

impl Default for SystemIcmpProbe {
    fn default() -> Self {
        Self {
            probe_count: DEFAULT_PROBE_COUNT,
            timeout: DEFAULT_PROBE_TIMEOUT,
            interval: DEFAULT_PROBE_INTERVAL,
        }
    }
}

#[async_trait]
impl IcmpProbe for SystemIcmpProbe {
    async fn observe(&self, address: IpAddr) -> NetworkLatencyObservation {
        let observed_at = now_rfc3339();
        let probe_count = self.probe_count;
        let timeout = self.timeout;
        let interval = self.interval;

        match tokio::task::spawn_blocking(move || {
            run_probes(address, probe_count, timeout, interval)
        })
        .await
        {
            Ok(samples) => observation_from_samples(address, observed_at, probe_count, samples),
            Err(error) => unavailable_observation(
                address,
                observed_at,
                probe_count,
                format!("ICMP probe task failed: {error}"),
            ),
        }
    }
}

fn run_probes(
    address: IpAddr,
    probe_count: u16,
    timeout: Duration,
    interval: Duration,
) -> Vec<Result<Duration, String>> {
    (0..probe_count)
        .map(|sequence| {
            if sequence > 0 {
                thread::sleep(interval);
            }
            ping::new(address)
                .socket_type(ping::RAW)
                .timeout(timeout)
                .seq_cnt(sequence.saturating_add(1))
                .send()
                .map(|reply| reply.rtt)
                .map_err(|error| error.to_string())
        })
        .collect()
}

fn observation_from_samples(
    address: IpAddr,
    observed_at: String,
    attempted_probes: u16,
    samples: Vec<Result<Duration, String>>,
) -> NetworkLatencyObservation {
    let last_error = samples
        .iter()
        .filter_map(|sample| sample.as_ref().err())
        .next_back()
        .cloned();
    let mut successful_micros = samples
        .into_iter()
        .filter_map(Result::ok)
        .map(duration_micros)
        .collect::<Vec<_>>();
    successful_micros.sort_unstable();

    let successful_probes = u32::try_from(successful_micros.len()).unwrap_or(u32::MAX);
    let attempted_probes = u32::from(attempted_probes);
    let packet_loss_percent = if attempted_probes == 0 {
        0.0
    } else {
        f64::from(attempted_probes - successful_probes) * 100.0 / f64::from(attempted_probes)
    };

    if successful_micros.is_empty() {
        return unavailable_observation(
            address,
            observed_at,
            u16::try_from(attempted_probes).unwrap_or(u16::MAX),
            last_error.unwrap_or_else(|| "no ICMP echo reply received".to_string()),
        );
    }

    NetworkLatencyObservation {
        protocol: NetworkLatencyProtocol::Icmp,
        status: NetworkLatencyStatus::Measured,
        observed_at,
        resolved_address: address.to_string(),
        attempted_probes,
        successful_probes,
        minimum_rtt_micros: successful_micros.first().copied(),
        median_rtt_micros: Some(median(&successful_micros)),
        packet_loss_percent,
        error: None,
    }
}

fn unavailable_observation(
    address: IpAddr,
    observed_at: String,
    attempted_probes: u16,
    error: String,
) -> NetworkLatencyObservation {
    NetworkLatencyObservation {
        protocol: NetworkLatencyProtocol::Icmp,
        status: NetworkLatencyStatus::Unavailable,
        observed_at,
        resolved_address: address.to_string(),
        attempted_probes: u32::from(attempted_probes),
        successful_probes: 0,
        minimum_rtt_micros: None,
        median_rtt_micros: None,
        packet_loss_percent: if attempted_probes == 0 { 0.0 } else { 100.0 },
        error: Some(error),
    }
}

fn duration_micros(duration: Duration) -> u64 {
    u64::try_from(duration.as_micros()).unwrap_or(u64::MAX)
}

fn median(sorted: &[u64]) -> u64 {
    let middle = sorted.len() / 2;
    if sorted.len() % 2 == 1 {
        sorted[middle]
    } else {
        let lower = u128::from(sorted[middle - 1]);
        let upper = u128::from(sorted[middle]);
        u64::try_from((lower + upper) / 2).unwrap_or(u64::MAX)
    }
}

fn now_rfc3339() -> String {
    OffsetDateTime::now_utc()
        .format(&Rfc3339)
        .unwrap_or_else(|_| OffsetDateTime::now_utc().to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn summarizes_successful_and_lost_probes() {
        let observation = observation_from_samples(
            "192.0.2.10".parse().unwrap(),
            "2026-09-22T12:00:00Z".to_string(),
            5,
            vec![
                Ok(Duration::from_micros(900)),
                Err("timeout".to_string()),
                Ok(Duration::from_micros(700)),
                Ok(Duration::from_micros(800)),
                Err("timeout".to_string()),
            ],
        );

        assert_eq!(observation.status, NetworkLatencyStatus::Measured);
        assert_eq!(observation.attempted_probes, 5);
        assert_eq!(observation.successful_probes, 3);
        assert_eq!(observation.minimum_rtt_micros, Some(700));
        assert_eq!(observation.median_rtt_micros, Some(800));
        assert_eq!(observation.packet_loss_percent, 40.0);
        assert_eq!(observation.error, None);
    }

    #[test]
    fn reports_unavailable_when_all_probes_fail() {
        let observation = observation_from_samples(
            "2001:db8::10".parse().unwrap(),
            "2026-09-22T12:00:00Z".to_string(),
            2,
            vec![Err("first".to_string()), Err("last".to_string())],
        );

        assert_eq!(observation.status, NetworkLatencyStatus::Unavailable);
        assert_eq!(observation.minimum_rtt_micros, None);
        assert_eq!(observation.median_rtt_micros, None);
        assert_eq!(observation.packet_loss_percent, 100.0);
        assert_eq!(observation.error.as_deref(), Some("last"));
    }

    #[test]
    fn calculates_even_sample_median_without_overflow() {
        assert_eq!(median(&[100, 200, 300, 400]), 250);
        assert_eq!(median(&[u64::MAX, u64::MAX]), u64::MAX);
    }
}
