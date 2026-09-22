use std::error::Error;
use std::sync::Arc;
use std::time::Duration;

use bmc_dispatch::Dispatcher;

use crate::config::Target;
use crate::mapper::{InventoryMapper, RedfishMapper};
use crate::models::NodeInventory;
use crate::source::icmp::{IcmpProbe, SystemIcmpProbe};
use crate::source::{InventorySource, RedfishSource};

/// Coordinates the complete inventory workflow for one BMC target.
///
/// The caller owns target-level concurrency. The shared dispatcher owned by
/// `RedfishSource` continues to control Redfish request concurrency and rate.
pub struct InventoryCollector {
    probe: Box<dyn IcmpProbe>,
    redfish: Box<dyn InventorySource>,
    mapper: RedfishMapper,
}

impl InventoryCollector {
    pub fn new(
        insecure_tls: bool,
        request_timeout: Duration,
        dispatcher: Arc<Dispatcher>,
    ) -> Result<Self, Box<dyn Error>> {
        Ok(Self {
            probe: Box::new(SystemIcmpProbe::default()),
            redfish: Box::new(RedfishSource::new(
                insecure_tls,
                request_timeout,
                dispatcher,
            )?),
            mapper: RedfishMapper::new(),
        })
    }

    pub async fn collect(&self, target: Target) -> NodeInventory {
        let network_latency = self.probe.observe(target.ip).await;
        let record = self.redfish.collect(target, network_latency).await;
        self.mapper.map(record).await
    }

    #[cfg(test)]
    fn with_components(
        probe: Box<dyn IcmpProbe>,
        redfish: Box<dyn InventorySource>,
        mapper: RedfishMapper,
    ) -> Self {
        Self {
            probe,
            redfish,
            mapper,
        }
    }
}

#[cfg(test)]
mod tests {
    use std::sync::atomic::{AtomicBool, Ordering};

    use async_trait::async_trait;
    use tokio::sync::Semaphore;

    use super::*;
    use crate::models::{
        NetworkLatencyObservation, NetworkLatencyProtocol, NetworkLatencyStatus, TargetKind,
    };
    use crate::source::{RedfishReport, ServiceRootData, SourceRecord};

    struct FixedProbe(NetworkLatencyObservation);

    #[async_trait]
    impl IcmpProbe for FixedProbe {
        async fn observe(&self, _address: std::net::IpAddr) -> NetworkLatencyObservation {
            self.0.clone()
        }
    }

    struct BlockingProbe {
        observation: NetworkLatencyObservation,
        started: Arc<Semaphore>,
        release: Arc<Semaphore>,
    }

    #[async_trait]
    impl IcmpProbe for BlockingProbe {
        async fn observe(&self, _address: std::net::IpAddr) -> NetworkLatencyObservation {
            self.started.add_permits(1);
            self.release.acquire().await.unwrap().forget();
            self.observation.clone()
        }
    }

    struct RecordingRedfish {
        called: Arc<AtomicBool>,
    }

    #[async_trait]
    impl InventorySource for RecordingRedfish {
        async fn collect(
            &self,
            target: Target,
            network_latency: NetworkLatencyObservation,
        ) -> SourceRecord {
            self.called.store(true, Ordering::SeqCst);
            SourceRecord {
                target,
                collected_at: "2026-09-22T17:00:00Z".to_string(),
                network_latency,
                result: Ok(RedfishReport {
                    service_root: ServiceRootData::default(),
                    managers: Vec::new(),
                    systems: Vec::new(),
                    chassis: Vec::new(),
                    firmware: Vec::new(),
                    errors: Vec::new(),
                }),
            }
        }
    }

    #[tokio::test]
    async fn redfish_waits_for_ping_completion() {
        let started = Arc::new(Semaphore::new(0));
        let release = Arc::new(Semaphore::new(0));
        let redfish_called = Arc::new(AtomicBool::new(false));
        let collector = Arc::new(InventoryCollector::with_components(
            Box::new(BlockingProbe {
                observation: measured_observation(),
                started: Arc::clone(&started),
                release: Arc::clone(&release),
            }),
            Box::new(RecordingRedfish {
                called: Arc::clone(&redfish_called),
            }),
            RedfishMapper::new(),
        ));

        let task = tokio::spawn({
            let collector = Arc::clone(&collector);
            async move { collector.collect(target()).await }
        });
        started.acquire().await.unwrap().forget();
        tokio::task::yield_now().await;
        assert!(!redfish_called.load(Ordering::SeqCst));

        release.add_permits(1);
        task.await.unwrap();
        assert!(redfish_called.load(Ordering::SeqCst));
    }

    #[tokio::test]
    async fn unavailable_ping_still_permits_redfish_collection() {
        let redfish_called = Arc::new(AtomicBool::new(false));
        let collector = InventoryCollector::with_components(
            Box::new(FixedProbe(unavailable_observation())),
            Box::new(RecordingRedfish {
                called: Arc::clone(&redfish_called),
            }),
            RedfishMapper::new(),
        );

        let inventory = collector.collect(target()).await;

        assert!(redfish_called.load(Ordering::SeqCst));
        assert!(inventory.target_error.is_none());
    }

    #[tokio::test]
    async fn latency_observation_reaches_final_inventory() {
        let expected = measured_observation();
        let collector = InventoryCollector::with_components(
            Box::new(FixedProbe(expected.clone())),
            Box::new(RecordingRedfish {
                called: Arc::new(AtomicBool::new(false)),
            }),
            RedfishMapper::new(),
        );

        let inventory = collector.collect(target()).await;

        assert_eq!(inventory.network_latency, expected);
    }

    fn target() -> Target {
        Target {
            name: Some("node-01".to_string()),
            kind: TargetKind::Compute,
            compute_zone: None,
            node_group: None,
            mac: "00:11:22:33:44:55".to_string(),
            ip: "127.0.0.1".parse().unwrap(),
            address: "127.0.0.1".to_string(),
            port: Some(443),
            username: "user".to_string(),
            password: "password".to_string(),
            labels: Default::default(),
        }
    }

    fn measured_observation() -> NetworkLatencyObservation {
        NetworkLatencyObservation {
            protocol: NetworkLatencyProtocol::Icmp,
            status: NetworkLatencyStatus::Measured,
            observed_at: "2026-09-22T16:59:59Z".to_string(),
            resolved_address: "127.0.0.1".to_string(),
            attempted_probes: 5,
            successful_probes: 5,
            minimum_rtt_micros: Some(100),
            median_rtt_micros: Some(120),
            packet_loss_percent: 0.0,
            error: None,
        }
    }

    fn unavailable_observation() -> NetworkLatencyObservation {
        NetworkLatencyObservation {
            status: NetworkLatencyStatus::Unavailable,
            attempted_probes: 5,
            successful_probes: 0,
            minimum_rtt_micros: None,
            median_rtt_micros: None,
            packet_loss_percent: 100.0,
            error: Some("ICMP echo reply timed out".to_string()),
            ..measured_observation()
        }
    }
}
