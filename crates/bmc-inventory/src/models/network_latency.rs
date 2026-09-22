use serde::Serialize;

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum NetworkLatencyProtocol {
    Icmp,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum NetworkLatencyStatus {
    Measured,
    Unavailable,
}

#[derive(Clone, Debug, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct NetworkLatencyObservation {
    pub protocol: NetworkLatencyProtocol,
    pub status: NetworkLatencyStatus,
    pub observed_at: String,
    pub resolved_address: String,
    pub attempted_probes: u32,
    pub successful_probes: u32,
    pub minimum_rtt_micros: Option<u64>,
    pub median_rtt_micros: Option<u64>,
    pub packet_loss_percent: f64,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
}

impl Default for NetworkLatencyObservation {
    fn default() -> Self {
        Self {
            protocol: NetworkLatencyProtocol::Icmp,
            status: NetworkLatencyStatus::Unavailable,
            observed_at: "1970-01-01T00:00:00Z".to_string(),
            resolved_address: String::new(),
            attempted_probes: 0,
            successful_probes: 0,
            minimum_rtt_micros: None,
            median_rtt_micros: None,
            packet_loss_percent: 0.0,
            error: Some("network latency was not measured".to_string()),
        }
    }
}
