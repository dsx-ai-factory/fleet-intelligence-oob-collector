use std::error::Error;
use std::time::Duration;

use base64::Engine;
use serde::{Deserialize, Serialize};
use time::OffsetDateTime;
use time::format_description::well_known::Rfc3339;
use tokio::time::sleep;
use url::Url;

const DEFAULT_SPDM_SLOT_ID: u8 = 0;

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
    ) -> SourceRecord {
        let collected_at = now_rfc3339();
        let nonce_refresh_timestamp = nonce.nonce_refresh_timestamp.clone();
        let result = self
            .collect_report(&target, &nonce, &request)
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
    ) -> Result<RedfishAttestationReport, Box<dyn Error>> {
        let base_url = bmc_url(&target.address, self.port)?;
        let components = self.discover_gpu_irot_components(&base_url, target).await?;
        if components.is_empty() {
            return Err("no GPU IRoT ComponentIntegrity resources found".into());
        }

        let mut evidences = Vec::new();
        let mut component_errors = Vec::new();
        for component in components {
            match self
                .collect_component(&base_url, target, &component, nonce, request)
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
        base_url: &Url,
        target: &crate::config::Target,
    ) -> Result<Vec<ComponentCandidate>, Box<dyn Error>> {
        let collection = self
            .get_json::<ComponentIntegrityCollection>(
                base_url,
                target,
                "/redfish/v1/ComponentIntegrity",
            )
            .await?;
        let mut out = Vec::new();
        for member in collection.members {
            let resource = self
                .get_json::<ComponentIntegrityResource>(base_url, target, &member.odata_id)
                .await?;
            if let Some(candidate) = component_candidate(member.odata_id, resource) {
                out.push(candidate);
            }
        }
        Ok(out)
    }

    async fn collect_component(
        &self,
        base_url: &Url,
        target: &crate::config::Target,
        component: &ComponentCandidate,
        nonce: &NonceData,
        request: &MeasurementRequest,
    ) -> Result<AttestationEvidenceData, Box<dyn Error>> {
        let task = self
            .post_json::<SpdmMeasurementBody, TaskResponse>(
                base_url,
                target,
                &component.action_target,
                &SpdmMeasurementBody {
                    slot_id: DEFAULT_SPDM_SLOT_ID,
                    measurement_indices: request.measurement_indices.clone(),
                    nonce: nonce.nonce.clone(),
                },
            )
            .await?;
        if let Some(task_path) = task.odata_id.as_deref() {
            self.wait_for_task(base_url, target, task_path).await?;
        }

        let data_path = format!("{}/data", component.action_target.trim_end_matches('/'));
        let data = self
            .get_json::<SpdmMeasurementData>(base_url, target, &data_path)
            .await?;
        if data.signed_measurements.is_empty() {
            return Err("SPDM measurement data missing SignedMeasurements".into());
        }

        let cert = self
            .get_json::<CertificateChain>(
                base_url,
                target,
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
        let gpu_context = self.gpu_context(base_url, target, &component.id).await;

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
        base_url: &Url,
        target: &crate::config::Target,
        task_path: &str,
    ) -> Result<(), Box<dyn Error>> {
        let deadline = std::time::Instant::now() + self.task_timeout;
        loop {
            let task = self
                .get_json::<TaskResponse>(base_url, target, task_path)
                .await?;
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
            if std::time::Instant::now() >= deadline {
                return Err(format!("SPDM task {task_path} timed out").into());
            }
            sleep(self.task_poll_interval).await;
        }
    }

    async fn gpu_context(
        &self,
        base_url: &Url,
        target: &crate::config::Target,
        component_id: &str,
    ) -> GpuContext {
        let mut context = GpuContext::default();
        let chassis = self
            .get_json::<ChassisContext>(
                base_url,
                target,
                &format!("/redfish/v1/Chassis/{component_id}"),
            )
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
        base_url: &Url,
        target: &crate::config::Target,
        path: &str,
    ) -> Result<T, Box<dyn Error>> {
        Ok(self
            .client
            .get(join_redfish_path(base_url, path)?)
            .basic_auth(&target.username, Some(&target.password))
            .send()
            .await?
            .error_for_status()?
            .json::<T>()
            .await?)
    }

    async fn post_json<B: Serialize, T: serde::de::DeserializeOwned>(
        &self,
        base_url: &Url,
        target: &crate::config::Target,
        path: &str,
        body: &B,
    ) -> Result<T, Box<dyn Error>> {
        Ok(self
            .client
            .post(join_redfish_path(base_url, path)?)
            .basic_auth(&target.username, Some(&target.password))
            .json(body)
            .send()
            .await?
            .error_for_status()?
            .json::<T>()
            .await?)
    }
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
}
