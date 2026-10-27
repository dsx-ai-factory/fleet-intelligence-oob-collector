// Copyright (c) 2025-2026, NVIDIA CORPORATION & AFFILIATES. All rights reserved.
//
// Licensed under the Apache License, Version 2.0 (the "License");
// you may not use this file except in compliance with the License.
// You may obtain a copy of the License at
//
//     http://www.apache.org/licenses/LICENSE-2.0
//
// Unless required by applicable law or agreed to in writing, software
// distributed under the License is distributed on an "AS IS" BASIS,
// WITHOUT WARRANTIES OR CONDITIONS OF ANY KIND, either express or implied.
// See the License for the specific language governing permissions and
// limitations under the License.
//
// SPDX-License-Identifier: Apache-2.0

use std::error::Error;
use std::time::Duration;

use base64::Engine;
use bmc_dispatch::RequestGate;
use serde::{Deserialize, Serialize};
use time::OffsetDateTime;
use time::format_description::well_known::Rfc3339;
use tokio::time::sleep;
use url::Url;

const DEFAULT_SPDM_SLOT_ID: u8 = 0;
/// Identifies collector traffic in BMC access logs.
const USER_AGENT: &str = "fleet-intelligence-oob-collector";

#[derive(Clone)]
pub struct RedfishAttestationSource {
    port: u16,
    task_timeout: Duration,
    task_poll_interval: Duration,
    client: reqwest::Client,
}

#[derive(Clone, Debug)]
pub struct MeasurementRequest {
    pub measurement_indices: Vec<u8>,
}

#[derive(Clone, Debug)]
pub struct NonceData {
    pub nonce: String,
    pub nonce_refresh_timestamp: Option<String>,
}

pub struct SourceRecord {
    pub target: crate::config::Target,
    pub collected_at: String,
    pub nonce_refresh_timestamp: Option<String>,
    pub result: Result<RedfishAttestationReport, String>,
}

impl SourceRecord {
    pub fn failed(
        target: crate::config::Target,
        nonce_refresh_timestamp: Option<String>,
        error: impl Into<String>,
    ) -> Self {
        Self {
            target,
            collected_at: now_rfc3339(),
            nonce_refresh_timestamp,
            result: Err(error.into()),
        }
    }
}

#[derive(Clone, Debug)]
pub struct RedfishAttestationReport {
    pub evidences: Vec<AttestationEvidenceData>,
    pub errors: Vec<ComponentErrorData>,
}

#[derive(Clone, Debug)]
pub struct AttestationEvidenceData {
    pub component_id: String,
    pub arch: Option<String>,
    pub vbios_version: Option<String>,
    pub nonce: String,
    pub evidence: String,
    pub certificate: String,
    pub version: Option<String>,
}

#[derive(Clone, Debug)]
pub struct ComponentErrorData {
    pub component_id: Option<String>,
    pub message: String,
}

#[derive(Clone, Debug)]
struct ComponentCandidate {
    id: String,
    action_target: String,
}

#[derive(Clone, Debug, Default)]
struct GpuContext {
    arch: Option<String>,
    vbios_version: Option<String>,
}

/// Resolves current credentials for a target whose identity and endpoint stay fixed.
pub type CredentialReloader<'a> =
    dyn Fn(&crate::config::Target) -> Result<(String, String), Box<dyn Error>> + Send + Sync + 'a;

struct BmcSession<'a> {
    target: crate::config::Target,
    base_url: Url,
    gate: RequestGate,
    reload_credentials: &'a CredentialReloader<'a>,
    credentials_reloaded: bool,
}

impl BmcSession<'_> {
    async fn request_json<T: serde::de::DeserializeOwned>(
        &mut self,
        request: impl Fn() -> reqwest::RequestBuilder,
    ) -> Result<T, Box<dyn Error>> {
        loop {
            let credentials = bmc_dispatch::BmcCredentials::new(
                self.target.username.clone(),
                self.target.password.clone(),
            );
            let result = self
                .gate
                .execute_classified(
                    &credentials,
                    async {
                        request()
                            .basic_auth(&self.target.username, Some(&self.target.password))
                            .send()
                            .await?
                            .error_for_status()?
                            .json::<T>()
                            .await
                    },
                    is_credential_rejection,
                )
                .await;
            match result {
                Ok(value) => return Ok(value),
                Err(bmc_dispatch::DispatchError::Transport(err))
                    if is_credential_rejection(&err) && !self.credentials_reloaded => {}
                Err(err) => return Err(err.into()),
            }
            self.credentials_reloaded = true;
            // Only credentials can change here. Retry this request with the same
            // nonce, retaining measurements already collected for other components.
            // Unchanged credentials stay rejected, so the retry fails locally.
            let (username, password) = (self.reload_credentials)(&self.target)
                .map_err(|_| "failed to reload BMC credentials")?;
            self.target.username = username;
            self.target.password = password;
            tracing::info!(bmc_mac = %self.target.bmc_mac, "BMC credentials reloaded; retrying authentication once");
        }
    }
}

impl RedfishAttestationSource {
    pub fn new(
        port: u16,
        insecure_tls: bool,
        request_timeout: Duration,
        task_timeout: Duration,
        task_poll_interval: Duration,
    ) -> Result<Self, Box<dyn Error>> {
        if request_timeout.is_zero() {
            return Err("request timeout must be greater than zero".into());
        }
        if task_timeout.is_zero() {
            return Err("task timeout must be greater than zero".into());
        }
        if task_poll_interval.is_zero() {
            return Err("task poll interval must be greater than zero".into());
        }
        let client = reqwest::Client::builder()
            .user_agent(USER_AGENT)
            .timeout(request_timeout)
            .connect_timeout(request_timeout.min(Duration::from_secs(10)))
            .danger_accept_invalid_certs(insecure_tls)
            .redirect(reqwest::redirect::Policy::none())
            .build()?;
        Ok(Self {
            port,
            task_timeout,
            task_poll_interval,
            client,
        })
    }

    pub async fn collect(
        &self,
        target: crate::config::Target,
        nonce: NonceData,
        request: MeasurementRequest,
        gate: RequestGate,
        reload_credentials: &CredentialReloader<'_>,
    ) -> SourceRecord {
        let collected_at = now_rfc3339();
        let nonce_refresh_timestamp = nonce.nonce_refresh_timestamp.clone();
        let result = self
            .collect_report(&target, &nonce, &request, gate, reload_credentials)
            .await
            .map_err(|err| err.to_string());
        SourceRecord {
            target,
            collected_at,
            nonce_refresh_timestamp,
            result,
        }
    }

    async fn collect_report(
        &self,
        target: &crate::config::Target,
        nonce: &NonceData,
        request: &MeasurementRequest,
        gate: RequestGate,
        reload_credentials: &CredentialReloader<'_>,
    ) -> Result<RedfishAttestationReport, Box<dyn Error>> {
        let mut session = BmcSession {
            target: target.clone(),
            base_url: bmc_url(&target.address, self.port)?,
            gate,
            reload_credentials,
            credentials_reloaded: false,
        };
        let components = self.discover_gpu_irot_components(&mut session).await?;
        if components.is_empty() {
            return Err("no GPU IRoT ComponentIntegrity resources found".into());
        }

        let mut evidences = Vec::new();
        let mut component_errors = Vec::new();
        for component in components {
            match self
                .collect_component(&mut session, &component, nonce, request)
                .await
            {
                Ok(evidence) => evidences.push(evidence),
                Err(err) => component_errors.push(ComponentErrorData {
                    component_id: Some(component.id),
                    message: err.to_string(),
                }),
            }
        }

        Ok(RedfishAttestationReport {
            evidences,
            errors: component_errors,
        })
    }

    async fn discover_gpu_irot_components(
        &self,
        session: &mut BmcSession<'_>,
    ) -> Result<Vec<ComponentCandidate>, Box<dyn Error>> {
        let collection = self
            .get_json::<ComponentIntegrityCollection>(session, "/redfish/v1/ComponentIntegrity")
            .await?;
        let mut out = Vec::new();
        for member in collection.members {
            let resource = self
                .get_json::<ComponentIntegrityResource>(session, &member.odata_id)
                .await?;
            if let Some(candidate) = component_candidate(member.odata_id, resource) {
                out.push(candidate);
            }
        }
        Ok(out)
    }

    async fn collect_component(
        &self,
        session: &mut BmcSession<'_>,
        component: &ComponentCandidate,
        nonce: &NonceData,
        request: &MeasurementRequest,
    ) -> Result<AttestationEvidenceData, Box<dyn Error>> {
        let task = self
            .post_json::<SpdmMeasurementBody, TaskResponse>(
                session,
                &component.action_target,
                &SpdmMeasurementBody {
                    slot_id: DEFAULT_SPDM_SLOT_ID,
                    measurement_indices: request.measurement_indices.clone(),
                    nonce: nonce.nonce.clone(),
                },
            )
            .await?;
        if let Some(task_path) = task.odata_id.as_deref() {
            self.wait_for_task(session, task_path).await?;
        }

        let data_path = format!("{}/data", component.action_target.trim_end_matches('/'));
        let data = self
            .get_json::<SpdmMeasurementData>(session, &data_path)
            .await?;
        if data.signed_measurements.is_empty() {
            return Err("SPDM measurement data missing SignedMeasurements".into());
        }

        let cert = self
            .get_json::<CertificateChain>(
                session,
                &format!(
                    "/redfish/v1/Chassis/{}/Certificates/CertChain",
                    component.id
                ),
            )
            .await?;
        if cert.certificate_string.is_empty() {
            return Err("certificate chain missing CertificateString".into());
        }
        let certificate = base64::engine::general_purpose::STANDARD.encode(cert.certificate_string);
        let gpu_context = self.gpu_context(session, &component.id).await;

        Ok(AttestationEvidenceData {
            component_id: component.id.clone(),
            arch: gpu_context.arch,
            vbios_version: gpu_context.vbios_version,
            nonce: nonce.nonce.clone(),
            evidence: data.signed_measurements,
            certificate,
            version: data.version,
        })
    }

    async fn wait_for_task(
        &self,
        session: &mut BmcSession<'_>,
        task_path: &str,
    ) -> Result<(), Box<dyn Error>> {
        let deadline = std::time::Instant::now() + self.task_timeout;
        loop {
            let remaining = deadline
                .checked_duration_since(std::time::Instant::now())
                .ok_or_else(|| format!("SPDM task {task_path} timed out"))?;
            let task =
                tokio::time::timeout(remaining, self.get_json::<TaskResponse>(session, task_path))
                    .await
                    .map_err(|_| format!("SPDM task {task_path} timed out"))??;
            if let Some(state) = task.task_state.as_deref() {
                if is_success_task_state(state) {
                    return Ok(());
                }
                if is_failed_task_state(state) {
                    return Err(format!(
                        "SPDM task {task_path} ended with state {state} and status {:?}",
                        task.task_status
                    )
                    .into());
                }
            }
            let remaining = deadline
                .checked_duration_since(std::time::Instant::now())
                .ok_or_else(|| format!("SPDM task {task_path} timed out"))?;
            sleep(self.task_poll_interval.min(remaining)).await;
        }
    }

    async fn gpu_context(&self, session: &mut BmcSession<'_>, component_id: &str) -> GpuContext {
        let mut context = GpuContext::default();
        let chassis = self
            .get_json::<ChassisContext>(session, &format!("/redfish/v1/Chassis/{component_id}"))
            .await
            .ok();
        if let Some(chassis) = chassis {
            context.vbios_version = chassis.firmware_version;
            context.arch = infer_arch([
                Some(component_id),
                chassis.name.as_deref(),
                chassis.model.as_deref(),
                chassis.sku.as_deref(),
                chassis.part_number.as_deref(),
            ]);
        }
        context
    }

    async fn get_json<T: serde::de::DeserializeOwned>(
        &self,
        session: &mut BmcSession<'_>,
        path: &str,
    ) -> Result<T, Box<dyn Error>> {
        let url = join_redfish_path(&session.base_url, path)?;
        session.request_json(|| self.client.get(url.clone())).await
    }

    async fn post_json<B: Serialize, T: serde::de::DeserializeOwned>(
        &self,
        session: &mut BmcSession<'_>,
        path: &str,
        body: &B,
    ) -> Result<T, Box<dyn Error>> {
        let url = join_redfish_path(&session.base_url, path)?;
        session
            .request_json(|| self.client.post(url.clone()).json(body))
            .await
    }
}

/// Only a 401 rejects the credentials; a 403 forbids one resource.
fn is_credential_rejection(error: &reqwest::Error) -> bool {
    error.status() == Some(reqwest::StatusCode::UNAUTHORIZED)
}

#[derive(Debug, Deserialize)]
struct OdataRef {
    #[serde(rename = "@odata.id")]
    odata_id: String,
}

#[derive(Debug, Deserialize)]
struct ComponentIntegrityCollection {
    #[serde(rename = "Members")]
    members: Vec<OdataRef>,
}

#[derive(Debug, Deserialize)]
struct ComponentIntegrityResource {
    #[serde(rename = "@odata.id")]
    odata_id: Option<String>,
    #[serde(rename = "Id")]
    id: Option<String>,
    #[serde(rename = "Name")]
    name: Option<String>,
    #[serde(rename = "Actions")]
    actions: Option<ComponentActions>,
}

#[derive(Debug, Deserialize)]
struct ComponentActions {
    #[serde(rename = "#ComponentIntegrity.SPDMGetSignedMeasurements")]
    spdm_get_signed_measurements: Option<ComponentAction>,
}

#[derive(Debug, Deserialize)]
struct ComponentAction {
    target: String,
}

#[derive(Debug, Serialize)]
struct SpdmMeasurementBody {
    #[serde(rename = "SlotId")]
    slot_id: u8,
    #[serde(rename = "MeasurementIndices")]
    measurement_indices: Vec<u8>,
    #[serde(rename = "Nonce")]
    nonce: String,
}

#[derive(Debug, Deserialize)]
struct TaskResponse {
    #[serde(rename = "@odata.id")]
    odata_id: Option<String>,
    #[serde(rename = "Id")]
    _id: Option<String>,
    #[serde(rename = "TaskState")]
    task_state: Option<String>,
    #[serde(rename = "TaskStatus")]
    task_status: Option<String>,
}

#[derive(Debug, Deserialize)]
struct SpdmMeasurementData {
    #[serde(rename = "SignedMeasurements")]
    signed_measurements: String,
    #[serde(rename = "Version")]
    version: Option<String>,
}

#[derive(Debug, Deserialize)]
struct CertificateChain {
    #[serde(rename = "CertificateString")]
    certificate_string: String,
}

#[derive(Debug, Deserialize)]
struct ChassisContext {
    #[serde(rename = "Name")]
    name: Option<String>,
    #[serde(rename = "Model")]
    model: Option<String>,
    #[serde(rename = "SKU")]
    sku: Option<String>,
    #[serde(rename = "PartNumber")]
    part_number: Option<String>,
    #[serde(rename = "FirmwareVersion")]
    firmware_version: Option<String>,
}

fn component_candidate(
    member_odata_id: String,
    resource: ComponentIntegrityResource,
) -> Option<ComponentCandidate> {
    let id = resource
        .id
        .clone()
        .or_else(|| member_odata_id.rsplit('/').next().map(ToString::to_string))?;
    let odata_id = resource.odata_id.unwrap_or(member_odata_id);
    if !contains_gpu_irot([
        Some(id.as_str()),
        resource.name.as_deref(),
        Some(odata_id.as_str()),
    ]) {
        return None;
    }
    let action_target = resource.actions?.spdm_get_signed_measurements?.target;
    Some(ComponentCandidate { id, action_target })
}

fn contains_gpu_irot<'a>(values: impl IntoIterator<Item = Option<&'a str>>) -> bool {
    let haystack = values
        .into_iter()
        .flatten()
        .collect::<Vec<_>>()
        .join(" ")
        .to_ascii_lowercase();
    haystack.contains("gpu") && haystack.contains("irot")
}

fn infer_arch<'a>(values: impl IntoIterator<Item = Option<&'a str>>) -> Option<String> {
    let haystack = values
        .into_iter()
        .flatten()
        .collect::<Vec<_>>()
        .join(" ")
        .to_ascii_lowercase();
    if haystack.contains("blackwell") || haystack.contains("gb200") || haystack.contains("b200") {
        return Some("BLACKWELL".to_string());
    }
    if haystack.contains("hopper") || haystack.contains("h100") || haystack.contains("h200") {
        return Some("HOPPER".to_string());
    }
    None
}

fn is_success_task_state(state: &str) -> bool {
    state.eq_ignore_ascii_case("Completed")
}

fn is_failed_task_state(state: &str) -> bool {
    matches!(
        state.to_ascii_lowercase().as_str(),
        "killed" | "exception" | "interrupted" | "cancelled" | "canceled"
    )
}

fn join_redfish_path(base_url: &Url, path: &str) -> Result<Url, Box<dyn Error>> {
    let mut url = base_url.clone();
    let mut segments = url
        .path_segments_mut()
        .map_err(|()| "BMC base URL cannot be a base")?;
    segments.clear();
    for segment in path.trim_start_matches('/').split('/') {
        if !segment.is_empty() {
            segments.push(segment);
        }
    }
    drop(segments);
    Ok(url)
}

fn bmc_url(address: &str, default_port: u16) -> Result<Url, Box<dyn Error>> {
    let mut url: Url = if address.starts_with("http://") || address.starts_with("https://") {
        address.parse()?
    } else {
        format!("https://{address}").parse()?
    };
    // Reject plaintext targets up front: attestation attaches Redfish Basic Auth
    // credentials, which must never be sent over an unencrypted http:// connection.
    if url.scheme() != "https" {
        return Err(format!(
            "BMC target {address:?} must use https://; plaintext http:// is not allowed because Redfish credentials would be sent in cleartext"
        )
        .into());
    }
    if url.port().is_none() {
        url.set_port(Some(default_port))
            .map_err(|()| format!("cannot set port {default_port} on {url}"))?;
    }
    Ok(url)
}

pub fn now_rfc3339() -> String {
    OffsetDateTime::now_utc()
        .format(&Rfc3339)
        .unwrap_or_else(|_| "1970-01-01T00:00:00Z".to_string())
}

#[cfg(test)]
mod tests {
    use std::convert::Infallible;

    use bmc_dispatch::{Dispatcher, DispatcherConfig};

    use super::*;

    #[test]
    fn gpu_irot_selector_is_case_insensitive() {
        assert!(contains_gpu_irot([
            Some("Baseboard GPU IRoT 0"),
            None,
            Some("/redfish/v1/ComponentIntegrity/Foo")
        ]));
        assert!(!contains_gpu_irot([Some("HGX_ERoT_CPU_0")]));
    }

    #[test]
    fn component_candidate_requires_gpu_irot_and_action() {
        let candidate = component_candidate(
            "/redfish/v1/ComponentIntegrity/Custom_0".to_string(),
            ComponentIntegrityResource {
                odata_id: None,
                id: Some("Custom_0".to_string()),
                name: Some("My GPU IRoT".to_string()),
                actions: Some(ComponentActions {
                    spdm_get_signed_measurements: Some(ComponentAction {
                        target: "/redfish/v1/ComponentIntegrity/Custom_0/Actions/ComponentIntegrity.SPDMGetSignedMeasurements".to_string(),
                    }),
                }),
            },
        )
        .expect("candidate");

        assert_eq!(candidate.id, "Custom_0");
        assert_eq!(
            candidate.action_target,
            "/redfish/v1/ComponentIntegrity/Custom_0/Actions/ComponentIntegrity.SPDMGetSignedMeasurements"
        );
    }

    #[test]
    fn infers_arch_from_redfish_inventory_context() {
        assert_eq!(
            infer_arch([Some("NVIDIA B200 SXM"), Some("GB200")]).as_deref(),
            Some("BLACKWELL")
        );
        assert_eq!(
            infer_arch([Some("NVIDIA H100 SXM")]).as_deref(),
            Some("HOPPER")
        );
    }

    #[test]
    fn bmc_url_adds_scheme_and_default_port() {
        let url = bmc_url("10.0.0.1", 443).expect("url");

        assert_eq!(url.as_str(), "https://10.0.0.1/");
        assert_eq!(url.port_or_known_default(), Some(443));
    }

    #[test]
    fn bmc_url_rejects_plaintext_http() {
        let error = bmc_url("http://10.0.0.1", 443)
            .expect_err("plaintext http should be rejected")
            .to_string();

        assert!(
            error.contains("must use https://"),
            "unexpected error: {error}"
        );
    }

    // Private HTTP fixtures exercise retry behavior; public collection still requires HTTPS.
    async fn read_request(socket: &mut tokio::net::TcpStream) -> String {
        use tokio::io::AsyncReadExt;
        let mut request = Vec::new();
        loop {
            let mut bytes = [0; 4096];
            let count = socket.read(&mut bytes).await.unwrap();
            assert!(count > 0, "incomplete request");
            request.extend_from_slice(&bytes[..count]);
            if let Some(end) = request.windows(4).position(|bytes| bytes == b"\r\n\r\n") {
                let headers = String::from_utf8_lossy(&request[..end]);
                let length = headers
                    .lines()
                    .filter_map(|line| line.split_once(':'))
                    .find(|(key, _)| key.eq_ignore_ascii_case("content-length"))
                    .map_or(0, |(_, value)| value.trim().parse::<usize>().unwrap());
                if request.len() >= end + 4 + length {
                    return String::from_utf8(request).unwrap();
                }
            }
        }
    }

    #[tokio::test]
    async fn rotated_credentials_retry_measurement_with_same_nonce_only_once() {
        use std::sync::atomic::{AtomicUsize, Ordering};
        use tokio::io::AsyncWriteExt;
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();
        let server = tokio::spawn(async move {
            // Rotate on the measurement POST, then reject a later request too.
            // Successful measurements survive; the second 401 cannot reload again.
            for (index, (status, body)) in [
                (401, "{}"),
                (200, "{}"),
                (200, r#"{"SignedMeasurements":"signed","Version":"1"}"#),
                (200, r#"{"CertificateString":"certificate"}"#),
                (200, "{}"),
                (401, "{}"),
            ]
            .into_iter()
            .enumerate()
            {
                let (mut socket, _) = listener.accept().await.unwrap();
                let request = read_request(&mut socket).await;
                let credentials = if index == 0 {
                    "user:old"
                } else {
                    "rotated:new"
                };
                let authorization = base64::engine::general_purpose::STANDARD.encode(credentials);
                let actual_auth = request
                    .lines()
                    .filter_map(|line| line.split_once(':'))
                    .find(|(key, _)| key.eq_ignore_ascii_case("authorization"))
                    .unwrap()
                    .1
                    .trim();
                assert_eq!(actual_auth, format!("Basic {authorization}"));
                if index < 2 {
                    assert!(request.starts_with("POST /measurements "));
                    assert!(request.contains(r#""Nonce":"same-nonce""#));
                }
                socket.write_all(format!("HTTP/1.1 {status} Test\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}", body.len()).as_bytes()).await.unwrap();
            }
        });
        let dispatcher = Dispatcher::start(
            DispatcherConfig {
                global_max_in_flight: 1,
                bmc_max_in_flight: 1,
                bmc_requests_per_second: 100,
                bmc_burst: 100,
                workflow_queue_capacity: 8,
            },
            ["bmc-1".to_string()],
        )
        .unwrap();
        let reload_count = AtomicUsize::new(0);
        let reload = |target: &crate::config::Target| {
            assert_eq!(target.bmc_mac, "00:11:22:33:44:55");
            assert_eq!(target.address, "192.0.2.1");
            assert_eq!(target.credentials_profile.as_deref(), Some("bmc"));
            reload_count.fetch_add(1, Ordering::Relaxed);
            Ok(("rotated".into(), "new".into()))
        };
        let mut session = BmcSession {
            target: crate::config::Target {
                name: "bmc-1".into(),
                bmc_mac: "00:11:22:33:44:55".into(),
                address: "192.0.2.1".into(),
                credentials_profile: Some("bmc".into()),
                username: "user".into(),
                password: "old".into(),
            },
            base_url: format!("http://{address}").parse().unwrap(),
            gate: dispatcher.attestation_gate("bmc-1").unwrap(),
            reload_credentials: &reload,
            credentials_reloaded: false,
        };
        let mut source = RedfishAttestationSource::new(
            443,
            false,
            Duration::from_secs(2),
            Duration::from_secs(2),
            Duration::from_millis(1),
        )
        .unwrap();
        source.client = reqwest::Client::builder()
            .no_proxy()
            .timeout(Duration::from_secs(2))
            .build()
            .unwrap();
        let evidence = source
            .collect_component(
                &mut session,
                &ComponentCandidate {
                    id: "GPU_0".into(),
                    action_target: "/measurements".into(),
                },
                &NonceData {
                    nonce: "same-nonce".into(),
                    nonce_refresh_timestamp: None,
                },
                &MeasurementRequest {
                    measurement_indices: vec![1],
                },
            )
            .await
            .unwrap();
        assert_eq!(evidence.nonce, "same-nonce");
        assert_eq!(evidence.evidence, "signed");
        assert!(
            source
                .get_json::<serde_json::Value>(&mut session, "/later")
                .await
                .is_err()
        );
        assert_eq!(reload_count.load(Ordering::Relaxed), 1);
        tokio::time::timeout(Duration::from_secs(3), server)
            .await
            .unwrap()
            .unwrap();
    }

    #[tokio::test]
    async fn auth_retry_is_bounded_and_credential_reload_errors_are_safe() {
        use std::sync::atomic::{AtomicUsize, Ordering};
        use tokio::io::AsyncWriteExt;
        // `reloaded` is the reloaded credentials, or `None` when reloading fails.
        // Reloaded credentials equal to the rejected ones are not sent again,
        // and a 401 stops inventory from sending the rejected credentials.
        for (status, reloaded, requests, reloads) in [
            (401, Some(("rotated", "new")), 2, 1),
            (401, Some(("user", "old")), 1, 1),
            (403, Some(("rotated", "new")), 1, 0),
            (401, None, 1, 1),
        ] {
            let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
            let address = listener.local_addr().unwrap();
            let server = tokio::spawn(async move {
                for _ in 0..requests {
                    let (mut socket, _) = listener.accept().await.unwrap();
                    read_request(&mut socket).await;
                    socket.write_all(format!("HTTP/1.1 {status} Test\r\nContent-Length: 0\r\nConnection: close\r\n\r\n").as_bytes()).await.unwrap();
                }
            });
            let dispatcher = Dispatcher::start(
                DispatcherConfig {
                    global_max_in_flight: 1,
                    bmc_max_in_flight: 1,
                    bmc_requests_per_second: 100,
                    bmc_burst: 100,
                    workflow_queue_capacity: 8,
                },
                ["bmc-1".into()],
            )
            .unwrap();
            let count = AtomicUsize::new(0);
            let reload = |_: &crate::config::Target| {
                count.fetch_add(1, Ordering::Relaxed);
                reloaded
                    .map(|(username, password)| (username.into(), password.into()))
                    .ok_or_else(|| "private-credential-detail".into())
            };
            let mut session = BmcSession {
                target: crate::config::Target {
                    name: "bmc-1".into(),
                    bmc_mac: "00:11:22:33:44:55".into(),
                    address: "192.0.2.1".into(),
                    credentials_profile: None,
                    username: "user".into(),
                    password: "old".into(),
                },
                base_url: format!("http://{address}").parse().unwrap(),
                gate: dispatcher.attestation_gate("bmc-1").unwrap(),
                reload_credentials: &reload,
                credentials_reloaded: false,
            };
            let client = reqwest::Client::builder()
                .no_proxy()
                .timeout(Duration::from_secs(2))
                .build()
                .unwrap();
            let error = session
                .request_json::<serde_json::Value>(|| {
                    client.get(format!("http://{address}/redfish/v1"))
                })
                .await
                .unwrap_err();
            assert!(!error.to_string().contains("private-credential-detail"));
            assert_eq!(count.load(Ordering::Relaxed), reloads);
            let inventory = dispatcher
                .inventory_gate("bmc-1")
                .unwrap()
                .execute_classified(
                    &bmc_dispatch::BmcCredentials::new("user".into(), "old".into()),
                    async { Ok::<_, Infallible>(()) },
                    |_| false,
                )
                .await;
            assert_eq!(
                matches!(
                    inventory,
                    Err(bmc_dispatch::DispatchError::CredentialsRejected)
                ),
                status == 401,
                "{status}"
            );
            tokio::time::timeout(Duration::from_secs(3), server)
                .await
                .unwrap()
                .unwrap();
        }
    }

    #[tokio::test]
    async fn task_poll_deadline_includes_dispatcher_wait() {
        let dispatcher = Dispatcher::start(
            DispatcherConfig {
                global_max_in_flight: 1,
                bmc_max_in_flight: 1,
                bmc_requests_per_second: 100,
                bmc_burst: 100,
                workflow_queue_capacity: 8,
            },
            ["bmc-1".to_string()],
        )
        .expect("dispatcher");
        let blocker = dispatcher.inventory_gate("bmc-1").expect("inventory gate");
        let poll_gate = dispatcher
            .attestation_gate("bmc-1")
            .expect("attestation gate");
        let (started_tx, started_rx) = tokio::sync::oneshot::channel();
        let (release_tx, release_rx) = tokio::sync::oneshot::channel();
        let blocked = tokio::spawn(async move {
            blocker
                .execute(async move {
                    let _ = started_tx.send(());
                    let _ = release_rx.await;
                    Ok::<_, Infallible>(())
                })
                .await
        });
        started_rx.await.expect("blocking request started");

        let source = RedfishAttestationSource::new(
            443,
            true,
            Duration::from_secs(1),
            Duration::from_millis(30),
            Duration::from_millis(5),
        )
        .expect("source");
        let target = crate::config::Target {
            name: "bmc-1".to_string(),
            bmc_mac: "00:11:22:33:44:55".to_string(),
            address: "192.0.2.1".to_string(),
            credentials_profile: Some("bmc".to_string()),
            username: "user".to_string(),
            password: "secret".to_string(),
        };
        let base_url = Url::parse("https://192.0.2.1").expect("base URL");
        let mut session = BmcSession {
            target,
            base_url,
            gate: poll_gate,
            reload_credentials: &|_| panic!("no HTTP request should start"),
            credentials_reloaded: false,
        };
        let started_at = std::time::Instant::now();
        let error = source
            .wait_for_task(&mut session, "/redfish/v1/TaskService/Tasks/1")
            .await
            .expect_err("dispatcher wait should consume the task deadline");

        assert!(error.to_string().contains("timed out"));
        assert!(started_at.elapsed() < Duration::from_millis(250));
        let _ = release_tx.send(());
        blocked.await.expect("blocking task").expect("request");
    }
}
