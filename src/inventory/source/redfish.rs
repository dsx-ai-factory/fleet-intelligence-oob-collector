use std::error::Error;
use std::sync::Arc;
use std::time::Duration;

use async_trait::async_trait;
use nv_redfish::bmc_http::{BmcCredentials, CacheSettings, HttpBmc};
use nv_redfish::core::ToSnakeCase;
use nv_redfish::{Bmc, Resource, ServiceRoot};
use time::OffsetDateTime;
use time::format_description::well_known::Rfc3339;
use url::Url;

use super::{
    ChassisData, ChassisLocationData, FirmwareData, InventoryErrorData, InventorySource,
    ManagerData, PcieDeviceData, RedfishReport, ServiceRootData, SourceRecord, SystemData,
};
use crate::config::Target;

type RedfishBmc = HttpBmc<nv_redfish::bmc_http::reqwest::Client>;

#[derive(Clone, Debug, PartialEq, Eq)]
enum PcieTraversalPolicy {
    Viking,
    SystemId(String),
    LenovoGb300,
    None,
}

pub struct RedfishSource {
    port: u16,
    client: nv_redfish::bmc_http::reqwest::Client,
}

impl RedfishSource {
    pub fn new(
        port: u16,
        insecure_tls: bool,
        request_timeout: Duration,
    ) -> Result<Self, Box<dyn Error>> {
        if request_timeout.is_zero() {
            return Err("request timeout must be greater than zero".into());
        }
        let connect_timeout = request_timeout.min(Duration::from_secs(10));
        let client = nv_redfish::bmc_http::reqwest::Client::with_params(
            nv_redfish::bmc_http::reqwest::ClientParams::new()
                .timeout(request_timeout)
                .connect_timeout(connect_timeout)
                .accept_invalid_certs(insecure_tls)
                .max_redirects(0),
        )?;
        Ok(Self { port, client })
    }

    async fn collect_report(&self, target: &Target) -> Result<RedfishReport, String> {
        let root = self
            .service_root(target)
            .await
            .map_err(|err| err.to_string())?;
        collect_report_from_root(&root).await
    }

    async fn service_root(
        &self,
        target: &Target,
    ) -> Result<ServiceRoot<RedfishBmc>, Box<dyn Error>> {
        let url = bmc_url(&target.address, self.port)?;
        let bmc = Arc::new(HttpBmc::new(
            self.client.clone(),
            url,
            BmcCredentials::new(target.username.clone(), target.password.clone()),
            CacheSettings::with_capacity(32),
        ));
        Ok(ServiceRoot::new(bmc).await?)
    }
}

#[async_trait]
impl InventorySource for RedfishSource {
    async fn collect(&self, target: Target) -> SourceRecord {
        let collected_at = now_rfc3339();
        let result = self.collect_report(&target).await;
        SourceRecord {
            target,
            collected_at,
            result,
        }
    }
}

async fn collect_report_from_root<B: Bmc>(root: &ServiceRoot<B>) -> Result<RedfishReport, String> {
    let raw_root = root.root.as_ref();
    let service_root = ServiceRootData {
        redfish_version: raw_root.redfish_version.clone(),
        uuid: raw_root.uuid.flatten().map(|uuid| uuid.to_string()),
        vendor: raw_root.vendor.clone().flatten(),
        product: raw_root.product.clone().flatten(),
    };

    let mut errors = Vec::new();
    let managers = match collect_managers(root).await {
        Ok(managers) => managers,
        Err(err) => {
            push_inventory_error(&mut errors, "managers", None, err);
            Vec::new()
        }
    };
    let systems = match collect_systems(root, &mut errors).await {
        Ok(systems) => systems,
        Err(err) => {
            push_inventory_error(&mut errors, "systems", None, err);
            Vec::new()
        }
    };
    let chassis = match collect_chassis(root, &service_root, &systems, &mut errors).await {
        Ok(chassis) => chassis,
        Err(err) => {
            push_inventory_error(&mut errors, "chassis", None, err);
            Vec::new()
        }
    };
    let firmware = match collect_firmware(root).await {
        Ok(firmware) => firmware,
        Err(err) => {
            push_inventory_error(&mut errors, "firmware", None, err);
            Vec::new()
        }
    };

    Ok(RedfishReport {
        service_root,
        managers,
        systems,
        chassis,
        firmware,
        errors,
    })
}

async fn collect_managers<B: Bmc>(root: &ServiceRoot<B>) -> Result<Vec<ManagerData>, String> {
    let Some(collection) = root
        .managers()
        .await
        .map_err(|err| format!("managers collection: {err}"))?
    else {
        return Ok(Vec::new());
    };

    let managers = collection
        .members()
        .await
        .map_err(|err| format!("manager members: {err}"))?;
    let mut out = Vec::with_capacity(managers.len());
    for manager in managers {
        let raw = manager.raw();
        out.push(ManagerData {
            id: manager.id().to_string(),
            odata_id: Some(manager.odata_id().to_string()),
            uuid: raw.uuid.flatten().map(|uuid| uuid.to_string()),
            manager_type: raw
                .manager_type
                .map(|manager_type| manager_type.to_snake_case().to_string()),
            model: raw.model.clone().flatten(),
            firmware_version: raw.firmware_version.clone().flatten(),
            status_state: status_state(raw.status.as_ref()),
            health: status_health(raw.status.as_ref()),
            health_rollup: status_health_rollup(raw.status.as_ref()),
        });
    }
    Ok(out)
}

async fn collect_systems<B: Bmc>(
    root: &ServiceRoot<B>,
    errors: &mut Vec<InventoryErrorData>,
) -> Result<Vec<SystemData>, String> {
    let Some(collection) = root
        .systems()
        .await
        .map_err(|err| format!("systems collection: {err}"))?
    else {
        return Ok(Vec::new());
    };

    let systems = collection
        .members()
        .await
        .map_err(|err| format!("system members: {err}"))?;
    let mut out = Vec::with_capacity(systems.len());
    for system in systems {
        let system_id = system.id().to_string();
        let raw = system.raw();
        out.push(SystemData {
            id: system_id.clone(),
            odata_id: Some(system.odata_id().to_string()),
            uuid: raw.uuid.flatten().map(|uuid| uuid.to_string()),
            manufacturer: raw.manufacturer.clone().flatten(),
            model: raw.model.clone().flatten(),
            sku: raw.sku.clone().flatten(),
            serial_number: raw.serial_number.clone().flatten(),
            bios_version: raw.bios_version.clone().flatten(),
            host_name: raw.host_name.clone().flatten(),
            asset_tag: raw.asset_tag.clone().flatten(),
            power_state: raw
                .power_state
                .flatten()
                .map(|power_state| power_state.to_snake_case().to_string()),
            status_state: status_state(raw.status.as_ref()),
            health: status_health(raw.status.as_ref()),
            health_rollup: status_health_rollup(raw.status.as_ref()),
            cpu_count: raw
                .processor_summary
                .as_ref()
                .and_then(|summary| summary.count)
                .flatten(),
            cpu_core_count: raw
                .processor_summary
                .as_ref()
                .and_then(|summary| summary.core_count)
                .flatten(),
            cpu_model: raw
                .processor_summary
                .as_ref()
                .and_then(|summary| summary.model.clone())
                .flatten(),
            memory_gib: raw
                .memory_summary
                .as_ref()
                .and_then(|summary| summary.total_system_memory_gi_b)
                .flatten(),
            secure_boot_enabled: collect_secure_boot_enabled(&system, &system_id, errors).await,
        });
    }
    Ok(out)
}

async fn collect_chassis<B: Bmc>(
    root: &ServiceRoot<B>,
    service_root: &ServiceRootData,
    systems: &[SystemData],
    errors: &mut Vec<InventoryErrorData>,
) -> Result<Vec<ChassisData>, String> {
    let Some(collection) = root
        .chassis()
        .await
        .map_err(|err| format!("chassis collection: {err}"))?
    else {
        return Ok(Vec::new());
    };

    let chassis = collection
        .members()
        .await
        .map_err(|err| format!("chassis members: {err}"))?;
    let pcie_traversal_policy = pcie_traversal_policy(service_root, systems, &chassis);
    let mut out = Vec::with_capacity(chassis.len());
    for chassis in chassis {
        let chassis_id = chassis.id().to_string();
        let raw = chassis.raw();
        out.push(ChassisData {
            id: chassis_id.clone(),
            odata_id: Some(chassis.odata_id().to_string()),
            chassis_type: Some(raw.chassis_type.to_snake_case().to_string()),
            manufacturer: raw.manufacturer.clone().flatten(),
            model: raw.model.clone().flatten(),
            sku: raw.sku.clone().flatten(),
            serial_number: raw.serial_number.clone().flatten(),
            part_number: raw.part_number.clone().flatten(),
            asset_tag: raw.asset_tag.clone().flatten(),
            power_state: raw
                .power_state
                .flatten()
                .map(|power_state| power_state.to_snake_case().to_string()),
            status_state: status_state(raw.status.as_ref()),
            health: status_health(raw.status.as_ref()),
            health_rollup: status_health_rollup(raw.status.as_ref()),
            location: chassis_location(raw.location.as_ref()),
            pcie_devices: if should_collect_pcie_devices_for_chassis(
                &pcie_traversal_policy,
                &chassis_id,
            ) {
                collect_pcie_devices(&chassis, &chassis_id, errors).await
            } else {
                Vec::new()
            },
        });
    }
    Ok(out)
}

fn pcie_traversal_policy<B: Bmc>(
    service_root: &ServiceRootData,
    systems: &[SystemData],
    chassis: &[nv_redfish::chassis::Chassis<B>],
) -> PcieTraversalPolicy {
    let vendor = service_root.vendor.as_deref();
    let product = service_root.product.as_deref();
    let selected_system_id =
        selected_system_for_pcie_traversal(systems).map(|system| system.id.clone());

    if vendor_matches(vendor, "AMI") && selected_system_id.as_deref() == Some("DGX") {
        return PcieTraversalPolicy::Viking;
    }
    if vendor_matches(vendor, "AMI") && is_lenovo_gb300_chassis_set(chassis) {
        return PcieTraversalPolicy::LenovoGb300;
    }
    if vendor_matches(vendor, "AMI") {
        return selected_system_id
            .map(PcieTraversalPolicy::SystemId)
            .unwrap_or(PcieTraversalPolicy::None);
    }
    if vendor_matches(vendor, "Dell")
        || vendor_matches(vendor, "HPE")
        || vendor_matches(vendor, "Supermicro")
        || vendor_matches(vendor, "Lenovo")
    {
        return selected_system_id
            .map(PcieTraversalPolicy::SystemId)
            .unwrap_or(PcieTraversalPolicy::None);
    }
    if vendor_matches(vendor, "Nvidia") && selected_system_id.as_deref() == Some("Bluefield") {
        return PcieTraversalPolicy::None;
    }
    if (vendor_matches(vendor, "WIWYNN") || vendor_matches(vendor, "NVIDIA"))
        && matches!(product, Some("GB200 NVL" | "GB BMC"))
    {
        return PcieTraversalPolicy::None;
    }
    if vendor_matches(vendor, "NVIDIA") && product == Some("P3809") {
        return PcieTraversalPolicy::None;
    }
    if is_liteon_powershelf_chassis_set(chassis) {
        return PcieTraversalPolicy::None;
    }

    PcieTraversalPolicy::None
}

fn selected_system_for_pcie_traversal(systems: &[SystemData]) -> Option<&SystemData> {
    systems
        .iter()
        .find(|system| system.bios_version.is_some())
        .or_else(|| systems.first())
}

fn should_collect_pcie_devices_for_chassis(policy: &PcieTraversalPolicy, chassis_id: &str) -> bool {
    match policy {
        PcieTraversalPolicy::Viking => {
            chassis_id.starts_with("HGX_GPU_SXM") || chassis_id.starts_with("HGX_NVSwitch")
        }
        PcieTraversalPolicy::SystemId(system_id) => chassis_id == system_id,
        PcieTraversalPolicy::LenovoGb300 => chassis_id.starts_with("HGX_GPU_"),
        PcieTraversalPolicy::None => false,
    }
}

fn vendor_matches(vendor: Option<&str>, expected: &str) -> bool {
    vendor.is_some_and(|vendor| vendor.eq_ignore_ascii_case(expected))
}

fn is_lenovo_gb300_chassis_set<B: Bmc>(chassis: &[nv_redfish::chassis::Chassis<B>]) -> bool {
    let has_gb300 = chassis.iter().any(|chassis| {
        let raw = chassis.raw();
        raw.manufacturer
            .as_ref()
            .and_then(|manufacturer| manufacturer.as_deref())
            .is_some_and(|manufacturer| manufacturer == "NVIDIA")
            && raw
                .model
                .as_ref()
                .and_then(|model| model.as_deref())
                .is_some_and(|model| model == "NVIDIA GB300")
    });
    let has_lenovo = chassis.iter().any(|chassis| {
        chassis
            .raw()
            .manufacturer
            .as_ref()
            .and_then(|manufacturer| manufacturer.as_deref())
            .is_some_and(|manufacturer| manufacturer == "Lenovo")
    });
    has_gb300 && has_lenovo
}

fn is_liteon_powershelf_chassis_set<B: Bmc>(chassis: &[nv_redfish::chassis::Chassis<B>]) -> bool {
    chassis.iter().any(|chassis| {
        let raw = chassis.raw();
        let chassis_id = chassis.id().to_string();
        chassis_id == "powershelf"
            || (chassis_id == "chassis"
                && raw
                    .manufacturer
                    .as_ref()
                    .and_then(|manufacturer| manufacturer.as_deref())
                    .is_some_and(|manufacturer| manufacturer.to_lowercase().contains("lite-on")))
    })
}

fn chassis_location(
    location: Option<&nv_redfish::schema::resource::Location>,
) -> Option<ChassisLocationData> {
    let location = location?;
    let part = location.part_location.as_ref();
    let placement = location.placement.as_ref();
    let data = ChassisLocationData {
        service_label: part.and_then(|p| p.service_label.clone().flatten()),
        rack: placement.and_then(|p| p.rack.clone().flatten()),
        row: placement.and_then(|p| p.row.clone().flatten()),
        room: placement.and_then(|p| p.room.clone().flatten()),
        rack_offset: placement.and_then(|p| p.rack_offset.flatten()),
    };
    if data.service_label.is_none()
        && data.rack.is_none()
        && data.row.is_none()
        && data.room.is_none()
        && data.rack_offset.is_none()
    {
        None
    } else {
        Some(data)
    }
}

async fn collect_secure_boot_enabled<B: Bmc>(
    system: &nv_redfish::computer_system::ComputerSystem<B>,
    system_id: &str,
    errors: &mut Vec<InventoryErrorData>,
) -> Option<bool> {
    let secure_boot = match system.secure_boot().await {
        Ok(secure_boot) => secure_boot?,
        Err(err) => {
            push_inventory_error(
                errors,
                "systems.secure_boot",
                Some(system_id.to_string()),
                err,
            );
            return None;
        }
    };
    secure_boot.secure_boot_enable()
}

async fn collect_pcie_devices<B: Bmc>(
    chassis: &nv_redfish::chassis::Chassis<B>,
    chassis_id: &str,
    errors: &mut Vec<InventoryErrorData>,
) -> Vec<PcieDeviceData> {
    let devices = match chassis.pcie_devices().await {
        Ok(Some(collection)) => match collection.members().await {
            Ok(devices) => devices,
            Err(err) => {
                push_inventory_error(
                    errors,
                    "chassis.pcie_devices",
                    Some(chassis_id.to_string()),
                    err,
                );
                Vec::new()
            }
        },
        Ok(None) => Vec::new(),
        Err(err) => {
            push_inventory_error(
                errors,
                "chassis.pcie_devices",
                Some(chassis_id.to_string()),
                err,
            );
            return Vec::new();
        }
    };

    let mut out = Vec::with_capacity(devices.len());
    for device in devices {
        let raw = device.raw();
        out.push(PcieDeviceData {
            id: device.id().to_string(),
            odata_id: Some(device.odata_id().to_string()),
            uuid: raw.uuid.flatten().map(|uuid| uuid.to_string()),
            device_type: raw
                .device_type
                .map(|device_type| device_type.to_snake_case().to_string()),
            manufacturer: raw.manufacturer.clone().flatten(),
            model: raw.model.clone().flatten(),
            sku: raw.sku.clone().flatten(),
            serial_number: raw.serial_number.clone().flatten(),
            part_number: raw.part_number.clone().flatten(),
            firmware_version: raw.firmware_version.clone().flatten(),
            status_state: status_state(raw.status.as_ref()),
            health: status_health(raw.status.as_ref()),
            health_rollup: status_health_rollup(raw.status.as_ref()),
        });
    }
    out
}

async fn collect_firmware<B: Bmc>(root: &ServiceRoot<B>) -> Result<Vec<FirmwareData>, String> {
    let Some(update_service) = root
        .update_service()
        .await
        .map_err(|err| format!("update service: {err}"))?
    else {
        return Ok(Vec::new());
    };
    let inventories = update_service
        .firmware_inventories()
        .await
        .map_err(|err| format!("firmware inventories: {err}"))?
        .unwrap_or_default();

    Ok(inventories
        .into_iter()
        .map(|inventory| {
            let raw = inventory.raw();
            FirmwareData {
                service_id: "FirmwareInventory".to_string(),
                id: inventory.id().to_string(),
                odata_id: Some(inventory.odata_id().to_string()),
                name: raw.base.name.clone(),
                version: raw.version.clone().flatten(),
                release_date: raw.release_date.flatten().map(|date| date.to_string()),
            }
        })
        .collect())
}

fn status_state(status: Option<&nv_redfish::schema::resource::Status>) -> Option<String> {
    status
        .and_then(|status| status.state.flatten())
        .and_then(redfish_enum_string)
}

fn status_health(status: Option<&nv_redfish::schema::resource::Status>) -> Option<String> {
    status
        .and_then(|status| status.health.flatten())
        .and_then(redfish_health_string)
}

fn status_health_rollup(status: Option<&nv_redfish::schema::resource::Status>) -> Option<String> {
    status
        .and_then(|status| status.health_rollup.flatten())
        .and_then(redfish_health_string)
}

fn redfish_health_string(health: nv_redfish::schema::resource::Health) -> Option<String> {
    match health {
        nv_redfish::schema::resource::Health::Ok => Some("OK".to_string()),
        nv_redfish::schema::resource::Health::Warning => Some("Warning".to_string()),
        nv_redfish::schema::resource::Health::Critical => Some("Critical".to_string()),
        nv_redfish::schema::resource::Health::UnsupportedValue => None,
    }
}

fn redfish_enum_string<T: serde::Serialize>(value: T) -> Option<String> {
    serde_json::to_value(value)
        .ok()
        .and_then(|value| value.as_str().map(ToString::to_string))
}

fn push_inventory_error(
    errors: &mut Vec<InventoryErrorData>,
    domain: &str,
    resource: Option<String>,
    error: impl std::fmt::Display,
) {
    errors.push(InventoryErrorData {
        domain: domain.to_string(),
        resource,
        message: error.to_string(),
    });
}

fn now_rfc3339() -> String {
    OffsetDateTime::now_utc()
        .format(&Rfc3339)
        .unwrap_or_else(|_| "1970-01-01T00:00:00Z".to_string())
}

fn bmc_url(address: &str, default_port: u16) -> Result<Url, Box<dyn Error>> {
    let mut url: Url = if address.starts_with("http://") || address.starts_with("https://") {
        address.parse()?
    } else {
        format!("https://{address}").parse()?
    };
    if url.port().is_none() {
        url.set_port(Some(default_port))
            .map_err(|()| format!("cannot set port {default_port} on {url}"))?;
    }
    Ok(url)
}

#[cfg(test)]
mod tests {
    use std::convert::Infallible;

    use super::*;
    use nv_redfish_bmc_mock::Expect;

    #[test]
    fn redfish_source_builds_client() {
        let source = RedfishSource::new(8443, true, Duration::from_secs(30)).expect("source");

        assert_eq!(source.port, 8443);
    }

    #[test]
    fn redfish_source_rejects_zero_timeout() {
        let Err(error) = RedfishSource::new(8443, true, Duration::ZERO) else {
            panic!("zero timeout should fail");
        };

        assert!(!error.to_string().is_empty());
    }

    #[test]
    fn bmc_url_adds_https_and_default_port_to_bare_address() {
        let url = bmc_url("10.0.0.1", 443).expect("url");

        assert_eq!(url.as_str(), "https://10.0.0.1/");
        assert_eq!(url.port_or_known_default(), Some(443));
    }

    #[test]
    fn bmc_url_keeps_explicit_port() {
        let url = bmc_url("https://bmc.example.com:8443", 443).expect("url");

        assert_eq!(url.as_str(), "https://bmc.example.com:8443/");
    }

    #[test]
    fn bmc_url_applies_default_port_to_scheme_only_url() {
        let url = bmc_url("http://bmc.example.com", 8443).expect("url");

        assert_eq!(url.as_str(), "http://bmc.example.com:8443/");
    }

    #[test]
    fn bmc_url_rejects_invalid_explicit_url() {
        let error = bmc_url("https://", 443).expect_err("invalid url");

        assert!(!error.to_string().is_empty());
    }

    #[test]
    fn pcie_traversal_policy_controls_selected_chassis() {
        assert!(should_collect_pcie_devices_for_chassis(
            &PcieTraversalPolicy::Viking,
            "HGX_GPU_SXM_0"
        ));
        assert!(should_collect_pcie_devices_for_chassis(
            &PcieTraversalPolicy::Viking,
            "HGX_NVSwitch_0"
        ));
        assert!(!should_collect_pcie_devices_for_chassis(
            &PcieTraversalPolicy::Viking,
            "HGX_GPU_PCIE_0"
        ));

        assert!(should_collect_pcie_devices_for_chassis(
            &PcieTraversalPolicy::SystemId("System".to_string()),
            "System"
        ));
        assert!(!should_collect_pcie_devices_for_chassis(
            &PcieTraversalPolicy::SystemId("System".to_string()),
            "BMC_0"
        ));

        assert!(should_collect_pcie_devices_for_chassis(
            &PcieTraversalPolicy::LenovoGb300,
            "HGX_GPU_PCIE_0"
        ));
        assert!(!should_collect_pcie_devices_for_chassis(
            &PcieTraversalPolicy::LenovoGb300,
            "HGX_NVSwitch_0"
        ));
        assert!(!should_collect_pcie_devices_for_chassis(
            &PcieTraversalPolicy::None,
            "HGX_GPU_SXM_0"
        ));
    }

    #[tokio::test]
    async fn collect_report_from_mock_bmc_traverses_inventory() {
        type MockBmc = nv_redfish_bmc_mock::Bmc<Infallible>;

        let bmc = Arc::new(MockBmc::default());
        bmc.expect(Expect::get("/redfish/v1", SERVICE_ROOT));
        bmc.expect(Expect::get("/redfish/v1/Managers", MANAGERS_COLLECTION));
        bmc.expect(Expect::get("/redfish/v1/Managers/BMC_0", MANAGER));
        bmc.expect(Expect::get("/redfish/v1/Systems", SYSTEMS_COLLECTION));
        bmc.expect(Expect::get("/redfish/v1/Systems/System", SYSTEM));
        bmc.expect(Expect::get("/redfish/v1/Chassis", CHASSIS_COLLECTION));
        bmc.expect(Expect::get("/redfish/v1/Chassis/HGX_GPU_SXM_0", CHASSIS));
        bmc.expect(Expect::get(
            "/redfish/v1/Chassis/HGX_GPU_SXM_0/PCIeDevices",
            PCIE_DEVICES_COLLECTION,
        ));
        bmc.expect(Expect::get(
            "/redfish/v1/Chassis/HGX_GPU_SXM_0/PCIeDevices/GPU1",
            PCIE_DEVICE,
        ));
        bmc.expect(Expect::get("/redfish/v1/UpdateService", UPDATE_SERVICE));
        bmc.expect(Expect::get(
            "/redfish/v1/UpdateService/FirmwareInventory",
            FIRMWARE_COLLECTION,
        ));
        bmc.expect(Expect::get(
            "/redfish/v1/UpdateService/FirmwareInventory/BMC",
            FIRMWARE,
        ));

        let root = ServiceRoot::new(Arc::clone(&bmc))
            .await
            .expect("service root wrapper");

        let report = collect_report_from_root(&root).await.expect("report");

        assert_eq!(
            report.service_root.redfish_version.as_deref(),
            Some("1.15.1")
        );
        assert_eq!(report.service_root.uuid.as_deref(), Some(SERVICE_UUID));
        assert_eq!(report.service_root.vendor.as_deref(), Some("AMI"));
        assert_eq!(report.service_root.product.as_deref(), Some("Mock BMC"));

        assert_eq!(report.managers.len(), 1);
        assert_eq!(report.managers[0].id, "BMC_0");
        assert_eq!(
            report.managers[0].firmware_version.as_deref(),
            Some("3.19.00")
        );

        assert_eq!(report.systems.len(), 1);
        assert_eq!(report.systems[0].id, "DGX");
        assert_eq!(report.systems[0].uuid.as_deref(), Some(SYSTEM_UUID));
        assert_eq!(report.systems[0].manufacturer.as_deref(), Some("NVIDIA"));
        assert_eq!(report.systems[0].model.as_deref(), Some("GB200"));
        assert_eq!(report.systems[0].sku.as_deref(), Some("SKU-GB200"));
        assert_eq!(report.systems[0].serial_number.as_deref(), Some("SN123"));
        assert_eq!(report.systems[0].bios_version.as_deref(), Some("3A21"));
        assert_eq!(
            report.systems[0].host_name.as_deref(),
            Some("node-a.example.com")
        );
        assert_eq!(
            report.systems[0].asset_tag.as_deref(),
            Some("rack-12-tray-3")
        );
        assert_eq!(report.systems[0].power_state.as_deref(), Some("on"));
        assert_eq!(report.systems[0].cpu_count, Some(1));
        assert_eq!(report.systems[0].cpu_core_count, Some(72));
        assert_eq!(report.systems[0].cpu_model.as_deref(), Some("Grace"));
        assert_eq!(report.systems[0].memory_gib, Some(480.0));
        assert_eq!(report.systems[0].secure_boot_enabled, None);
        assert_eq!(report.systems[0].status_state.as_deref(), Some("Enabled"));
        assert_eq!(report.systems[0].health.as_deref(), Some("OK"));
        assert_eq!(report.systems[0].health_rollup.as_deref(), Some("Warning"));

        assert_eq!(report.chassis.len(), 1);
        assert_eq!(report.chassis[0].id, "HGX_GPU_SXM_0");
        assert_eq!(report.chassis[0].chassis_type.as_deref(), Some("module"));
        assert_eq!(report.chassis[0].manufacturer.as_deref(), Some("NVIDIA"));
        assert_eq!(report.chassis[0].model.as_deref(), Some("BMC"));
        assert_eq!(report.chassis[0].serial_number.as_deref(), Some("CH123"));
        assert_eq!(report.chassis[0].part_number.as_deref(), Some("PN123"));
        let location = report.chassis[0]
            .location
            .as_ref()
            .expect("chassis location");
        assert_eq!(location.service_label.as_deref(), Some("Bay 1"));
        assert_eq!(location.rack.as_deref(), Some("R12"));
        assert_eq!(location.row.as_deref(), Some("A"));
        assert_eq!(location.rack_offset, Some(7));

        assert_eq!(report.chassis[0].pcie_devices.len(), 1);
        assert_eq!(report.chassis[0].pcie_devices[0].id, "GPU1");
        assert_eq!(
            report.chassis[0].pcie_devices[0].serial_number.as_deref(),
            Some("GPU-SN456")
        );

        assert_eq!(report.firmware.len(), 1);
        assert_eq!(report.firmware[0].service_id, "FirmwareInventory");
        assert_eq!(report.firmware[0].id, "BMC");
        assert_eq!(report.firmware[0].name, "BMC");
        assert_eq!(report.firmware[0].version.as_deref(), Some("3.19.00"));
        assert_eq!(
            report.firmware[0].release_date.as_deref(),
            Some("2026-01-01T00:00:00Z")
        );
    }

    #[tokio::test]
    async fn manager_collection_failure_is_partial_and_keeps_other_inventory() {
        type MockBmc = nv_redfish_bmc_mock::Bmc<Infallible>;

        let bmc = Arc::new(MockBmc::default());
        bmc.expect(Expect::get("/redfish/v1", SERVICE_ROOT));
        bmc.expect(Expect::get("/redfish/v1/Managers", MANAGERS_COLLECTION));
        bmc.expect(Expect::get("/redfish/v1/Managers/BMC_0", MALFORMED_MANAGER));
        bmc.expect(Expect::get("/redfish/v1/Systems", SYSTEMS_COLLECTION));
        bmc.expect(Expect::get("/redfish/v1/Systems/System", SYSTEM));
        bmc.expect(Expect::get("/redfish/v1/Chassis", CHASSIS_COLLECTION));
        bmc.expect(Expect::get("/redfish/v1/Chassis/HGX_GPU_SXM_0", CHASSIS));
        bmc.expect(Expect::get(
            "/redfish/v1/Chassis/HGX_GPU_SXM_0/PCIeDevices",
            PCIE_DEVICES_COLLECTION,
        ));
        bmc.expect(Expect::get(
            "/redfish/v1/Chassis/HGX_GPU_SXM_0/PCIeDevices/GPU1",
            PCIE_DEVICE,
        ));
        bmc.expect(Expect::get("/redfish/v1/UpdateService", UPDATE_SERVICE));
        bmc.expect(Expect::get(
            "/redfish/v1/UpdateService/FirmwareInventory",
            FIRMWARE_COLLECTION,
        ));
        bmc.expect(Expect::get(
            "/redfish/v1/UpdateService/FirmwareInventory/BMC",
            FIRMWARE,
        ));

        let root = ServiceRoot::new(Arc::clone(&bmc))
            .await
            .expect("service root wrapper");

        let report = collect_report_from_root(&root).await.expect("report");

        assert!(report.managers.is_empty());
        assert_eq!(report.systems.len(), 1);
        assert_eq!(report.chassis.len(), 1);
        assert_eq!(report.chassis[0].pcie_devices.len(), 1);
        assert_eq!(report.firmware.len(), 1);
        assert_eq!(report.errors.len(), 1);
        assert_eq!(report.errors[0].domain, "managers");
        assert!(report.errors[0].resource.is_none());
        assert!(!report.errors[0].message.is_empty());
    }

    #[tokio::test]
    async fn firmware_collection_failure_is_partial_and_keeps_other_inventory() {
        type MockBmc = nv_redfish_bmc_mock::Bmc<Infallible>;

        let bmc = Arc::new(MockBmc::default());
        bmc.expect(Expect::get("/redfish/v1", SERVICE_ROOT));
        bmc.expect(Expect::get("/redfish/v1/Managers", MANAGERS_COLLECTION));
        bmc.expect(Expect::get("/redfish/v1/Managers/BMC_0", MANAGER));
        bmc.expect(Expect::get("/redfish/v1/Systems", SYSTEMS_COLLECTION));
        bmc.expect(Expect::get("/redfish/v1/Systems/System", SYSTEM));
        bmc.expect(Expect::get("/redfish/v1/Chassis", CHASSIS_COLLECTION));
        bmc.expect(Expect::get("/redfish/v1/Chassis/HGX_GPU_SXM_0", CHASSIS));
        bmc.expect(Expect::get(
            "/redfish/v1/Chassis/HGX_GPU_SXM_0/PCIeDevices",
            PCIE_DEVICES_COLLECTION,
        ));
        bmc.expect(Expect::get(
            "/redfish/v1/Chassis/HGX_GPU_SXM_0/PCIeDevices/GPU1",
            PCIE_DEVICE,
        ));
        bmc.expect(Expect::get("/redfish/v1/UpdateService", UPDATE_SERVICE));
        bmc.expect(Expect::get(
            "/redfish/v1/UpdateService/FirmwareInventory",
            FIRMWARE_COLLECTION,
        ));
        bmc.expect(Expect::get(
            "/redfish/v1/UpdateService/FirmwareInventory/BMC",
            MALFORMED_FIRMWARE,
        ));

        let root = ServiceRoot::new(Arc::clone(&bmc))
            .await
            .expect("service root wrapper");

        let report = collect_report_from_root(&root).await.expect("report");

        assert_eq!(report.managers.len(), 1);
        assert_eq!(report.systems.len(), 1);
        assert_eq!(report.chassis.len(), 1);
        assert_eq!(report.chassis[0].pcie_devices.len(), 1);
        assert!(report.firmware.is_empty());
        assert_eq!(report.errors.len(), 1);
        assert_eq!(report.errors[0].domain, "firmware");
        assert!(report.errors[0].resource.is_none());
        assert!(!report.errors[0].message.is_empty());
    }

    const SERVICE_UUID: &str = "870155dd-d474-aca8-11ee-6e809af87c0c";
    const SYSTEM_UUID: &str = "9aee71e4-6e80-11ee-a8ac-74d4dd550187";

    const SERVICE_ROOT: &str = r##"{
        "@odata.id": "/redfish/v1",
        "@odata.type": "#ServiceRoot.v1_15_0.ServiceRoot",
        "Id": "RootService",
        "Name": "Root Service",
        "RedfishVersion": "1.15.1",
        "UUID": "870155dd-d474-aca8-11ee-6e809af87c0c",
        "Links": {
            "Sessions": {"@odata.id": "/redfish/v1/SessionService/Sessions"}
        },
        "Vendor": "AMI",
        "Product": "Mock BMC",
        "Managers": {"@odata.id": "/redfish/v1/Managers"},
        "Systems": {"@odata.id": "/redfish/v1/Systems"},
        "Chassis": {"@odata.id": "/redfish/v1/Chassis"},
        "UpdateService": {"@odata.id": "/redfish/v1/UpdateService"}
    }"##;

    const MANAGERS_COLLECTION: &str = r##"{
        "@odata.id": "/redfish/v1/Managers",
        "@odata.type": "#ManagerCollection.ManagerCollection",
        "Id": "Managers",
        "Name": "Managers",
        "Members@odata.count": 1,
        "Members": [{"@odata.id": "/redfish/v1/Managers/BMC_0"}]
    }"##;

    const MANAGER: &str = r##"{
        "@odata.id": "/redfish/v1/Managers/BMC_0",
        "@odata.type": "#Manager.v1_15_0.Manager",
        "Id": "BMC_0",
        "Name": "BMC",
        "ManagerType": "BMC",
        "UUID": "11111111-2222-3333-4444-555555555555",
        "Model": "OpenBMC",
        "FirmwareVersion": "3.19.00"
    }"##;

    const MALFORMED_MANAGER: &str = r##"{
        "@odata.id": "/redfish/v1/Managers/BMC_0",
        "@odata.type": "#Manager.v1_15_0.Manager",
        "Id": "BMC_0",
        "Name": "BMC",
        "ManagerType": 1
    }"##;

    const SYSTEMS_COLLECTION: &str = r##"{
        "@odata.id": "/redfish/v1/Systems",
        "@odata.type": "#ComputerSystemCollection.ComputerSystemCollection",
        "Id": "Systems",
        "Name": "Systems",
        "Members@odata.count": 1,
        "Members": [{"@odata.id": "/redfish/v1/Systems/System"}]
    }"##;

    const SYSTEM: &str = r##"{
        "@odata.id": "/redfish/v1/Systems/System",
        "@odata.type": "#ComputerSystem.v1_15_0.ComputerSystem",
        "Id": "DGX",
        "Name": "System",
        "UUID": "9AEE71E4-6E80-11EE-A8AC-74D4DD550187",
        "Manufacturer": "NVIDIA",
        "Model": "GB200",
        "SKU": "SKU-GB200",
        "SerialNumber": "SN123",
        "BiosVersion": "3A21",
        "HostName": "node-a.example.com",
        "AssetTag": "rack-12-tray-3",
        "PowerState": "On",
        "ProcessorSummary": {
            "Count": 1,
            "CoreCount": 72,
            "Model": "Grace"
        },
        "MemorySummary": {
            "TotalSystemMemoryGiB": 480
        },
        "Status": {
            "State": "Enabled",
            "Health": "OK",
            "HealthRollup": "Warning"
        }
    }"##;

    const CHASSIS_COLLECTION: &str = r##"{
        "@odata.id": "/redfish/v1/Chassis",
        "@odata.type": "#ChassisCollection.ChassisCollection",
        "Id": "Chassis",
        "Name": "Chassis",
        "Members@odata.count": 1,
        "Members": [{"@odata.id": "/redfish/v1/Chassis/HGX_GPU_SXM_0"}]
    }"##;

    const CHASSIS: &str = r##"{
        "@odata.id": "/redfish/v1/Chassis/HGX_GPU_SXM_0",
        "@odata.type": "#Chassis.v1_25_0.Chassis",
        "Id": "HGX_GPU_SXM_0",
        "Name": "BMC Chassis",
        "ChassisType": "Module",
        "Manufacturer": "NVIDIA",
        "Model": "BMC",
        "SerialNumber": "CH123",
        "PartNumber": "PN123",
        "PCIeDevices": {"@odata.id": "/redfish/v1/Chassis/HGX_GPU_SXM_0/PCIeDevices"},
        "Location": {
            "PartLocation": {"ServiceLabel": "Bay 1"},
            "Placement": {"Rack": "R12", "Row": "A", "RackOffset": 7}
        }
    }"##;

    const PCIE_DEVICES_COLLECTION: &str = r##"{
        "@odata.id": "/redfish/v1/Chassis/HGX_GPU_SXM_0/PCIeDevices",
        "@odata.type": "#PCIeDeviceCollection.PCIeDeviceCollection",
        "Id": "PCIeDevices",
        "Name": "PCIe Devices",
        "Members@odata.count": 1,
        "Members": [{"@odata.id": "/redfish/v1/Chassis/HGX_GPU_SXM_0/PCIeDevices/GPU1"}]
    }"##;

    const PCIE_DEVICE: &str = r##"{
        "@odata.id": "/redfish/v1/Chassis/HGX_GPU_SXM_0/PCIeDevices/GPU1",
        "@odata.type": "#PCIeDevice.v1_16_0.PCIeDevice",
        "Id": "GPU1",
        "Name": "GPU 1",
        "Manufacturer": "NVIDIA",
        "Model": "B200",
        "SKU": "B200-SXM",
        "SerialNumber": "GPU-SN456",
        "PartNumber": "GPU-PN456",
        "DeviceType": "SingleFunction",
        "FirmwareVersion": "96.00.5E.00.02",
        "Status": {
            "State": "Enabled",
            "Health": "OK"
        }
    }"##;

    const UPDATE_SERVICE: &str = r##"{
        "@odata.id": "/redfish/v1/UpdateService",
        "@odata.type": "#UpdateService.v1_14_0.UpdateService",
        "Id": "UpdateService",
        "Name": "Update Service",
        "FirmwareInventory": {"@odata.id": "/redfish/v1/UpdateService/FirmwareInventory"}
    }"##;

    const FIRMWARE_COLLECTION: &str = r##"{
        "@odata.id": "/redfish/v1/UpdateService/FirmwareInventory",
        "@odata.type": "#SoftwareInventoryCollection.SoftwareInventoryCollection",
        "Id": "FirmwareInventory",
        "Name": "Firmware Inventory",
        "Members@odata.count": 1,
        "Members": [{"@odata.id": "/redfish/v1/UpdateService/FirmwareInventory/BMC"}]
    }"##;

    const FIRMWARE: &str = r##"{
        "@odata.id": "/redfish/v1/UpdateService/FirmwareInventory/BMC",
        "@odata.type": "#SoftwareInventory.v1_8_0.SoftwareInventory",
        "Id": "BMC",
        "Name": "BMC",
        "Version": "3.19.00",
        "ReleaseDate": "2026-01-01T00:00:00Z"
    }"##;

    const MALFORMED_FIRMWARE: &str = r##"{
        "@odata.id": "/redfish/v1/UpdateService/FirmwareInventory/BMC",
        "@odata.type": "#SoftwareInventory.v1_8_0.SoftwareInventory",
        "Id": 1,
        "Name": "BMC"
    }"##;
}
