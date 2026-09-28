use bmc_error::SanitizedErrorMessage;
use serde::Serialize;

/// How collector-to-BMC latency was measured.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum LatencyMethod {
    /// Time to establish a TCP connection to the Redfish port; no TLS or HTTP.
    TcpConnect,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum LatencyStatus {
    /// Enough attempts succeeded to report a round minimum.
    Ok,
    /// Too few attempts succeeded; the round carries no latency value.
    Unavailable,
    /// A local component answered TCP for an unroutable address, so connect
    /// timing does not reflect the BMC path and the round was skipped.
    Intercepted,
}

/// One measurement round from this collector to one BMC.
#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct LatencyMeasurement {
    pub method: LatencyMethod,
    pub status: LatencyStatus,
    pub destination: String,
    pub measured_at: String,
    pub attempts: u32,
    pub successes: u32,
    pub min_us: Option<u64>,
    pub median_us: Option<u64>,
    pub error: Option<SanitizedErrorMessage>,
}
