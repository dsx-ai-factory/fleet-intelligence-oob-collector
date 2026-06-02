use std::error::Error;
use std::sync::Arc;
use std::time::Duration;

use async_trait::async_trait;
use mac_address::MacAddress;
use nv_redfish::bmc_http::{BmcCredentials, CacheSettings, HttpBmc};
use nv_redfish::core::{ODataId, ToSnakeCase};
use nv_redfish::schema::service_root::ServiceRoot as ServiceRootSchema;
use nv_redfish::{Bmc, Resource, ServiceRoot};
use url::Url;

use super::{
    ChassisData, FirmwareData, InventorySource, ManagerData, RedfishReport, ServiceRootData,
    SourceRecord, SystemData,
};
use crate::config::Target;

type RedfishBmc = HttpBmc<nv_redfish::bmc_http::reqwest::Client>;

struct RootSession {
    root: ServiceRoot<RedfishBmc>,
    raw: Arc<ServiceRootSchema>,
}

pub struct RedfishSource {
    port: u16,
    insecure_tls: bool,
    _boot_mac: Option<MacAddress>,
}

impl RedfishSource {
    pub fn new(port: u16, insecure_tls: bool, boot_mac: Option<MacAddress>) -> Self {
        Self {
            port,
            insecure_tls,
            _boot_mac: boot_mac,
        }
    }

    async fn collect_report(&self, target: &Target) -> Result<RedfishReport, String> {
        let root_session = service_root(target, self.port, self.insecure_tls)
            .await
            .map_err(|err| err.to_string())?;
        collect_report_from_root(&root_session.root, &root_session.raw).await
    }
}

#[async_trait]
impl InventorySource for RedfishSource {
    async fn collect(&self, target: Target) -> SourceRecord {
        let result = self.collect_report(&target).await;
        SourceRecord { target, result }
    }
}

async fn collect_report_from_root<B: Bmc>(
    root: &ServiceRoot<B>,
    raw_root: &ServiceRootSchema,
) -> Result<RedfishReport, String> {
    let service_root = ServiceRootData {
        redfish_version: raw_root.redfish_version.clone(),
        uuid: raw_root.uuid.flatten().map(|uuid| uuid.to_string()),
        vendor: raw_root.vendor.clone().flatten(),
        product: raw_root.product.clone().flatten(),
    };

    let managers = collect_managers(root).await?;
    let systems = collect_systems(root).await?;
    let chassis = collect_chassis(root).await?;
    let firmware = collect_firmware(root).await?;

    Ok(RedfishReport {
        service_root,
        managers,
        systems,
        chassis,
        firmware,
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
        let interfaces = match manager.ethernet_interfaces().await {
            Ok(Some(collection)) => collection.members().await.unwrap_or_default(),
            Ok(None) | Err(_) => Vec::new(),
        };
        out.push(ManagerData {
            id: manager.id().to_string(),
            uuid: raw.uuid.clone().flatten().map(|uuid| uuid.to_string()),
            manager_type: raw
                .manager_type
                .map(|manager_type| manager_type.to_snake_case().to_string()),
            model: raw.model.clone().flatten(),
            firmware_version: raw.firmware_version.clone().flatten(),
            interface_count: interfaces.len(),
        });
    }
    Ok(out)
}

async fn collect_systems<B: Bmc>(root: &ServiceRoot<B>) -> Result<Vec<SystemData>, String> {
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
        let raw = system.raw();
        let interfaces = match system.ethernet_interfaces().await {
            Ok(Some(collection)) => collection.members().await.unwrap_or_default(),
            Ok(None) | Err(_) => Vec::new(),
        };
        out.push(SystemData {
            id: system.id().to_string(),
            uuid: raw.uuid.clone().flatten().map(|uuid| uuid.to_string()),
            manufacturer: raw.manufacturer.clone().flatten(),
            model: raw.model.clone().flatten(),
            serial_number: raw.serial_number.clone().flatten(),
            bios_version: raw.bios_version.clone().flatten(),
            power_state: raw
                .power_state
                .flatten()
                .map(|power_state| power_state.to_snake_case().to_string()),
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
            interface_count: interfaces.len(),
            pcie_device_count: 0,
        });
    }
    Ok(out)
}

async fn collect_chassis<B: Bmc>(root: &ServiceRoot<B>) -> Result<Vec<ChassisData>, String> {
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
    let mut out = Vec::with_capacity(chassis.len());
    for chassis in chassis {
        let raw = chassis.raw();
        let network_adapters = match chassis.network_adapters().await {
            Ok(Some(adapters)) => adapters,
            Ok(None) | Err(_) => Vec::new(),
        };
        out.push(ChassisData {
            id: chassis.id().to_string(),
            chassis_type: Some(raw.chassis_type.to_snake_case().to_string()),
            manufacturer: raw.manufacturer.clone().flatten(),
            model: raw.model.clone().flatten(),
            serial_number: raw.serial_number.clone().flatten(),
            part_number: raw.part_number.clone().flatten(),
            network_adapter_count: network_adapters.len(),
        });
    }
    Ok(out)
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
                name: raw.base.name.clone(),
                version: raw.version.clone().flatten(),
                release_date: raw.release_date.flatten().map(|date| date.to_string()),
            }
        })
        .collect())
}

async fn service_root(
    target: &Target,
    default_port: u16,
    insecure_tls: bool,
) -> Result<RootSession, Box<dyn Error>> {
    let url = bmc_url(&target.address, default_port)?;
    let client = nv_redfish::bmc_http::reqwest::Client::with_params(
        nv_redfish::bmc_http::reqwest::ClientParams::new()
            .timeout(Duration::from_secs(10))
            .connect_timeout(Duration::from_secs(5))
            .accept_invalid_certs(insecure_tls)
            .max_redirects(0),
    )?;
    let bmc = Arc::new(HttpBmc::new(
        client,
        url,
        BmcCredentials::new(target.username.clone(), target.password.clone()),
        CacheSettings::with_capacity(32),
    ));
    let raw = bmc
        .get::<ServiceRootSchema>(&ODataId::service_root())
        .await?;
    let root = ServiceRoot::new(bmc).await?;
    Ok(RootSession { root, raw })
}

fn bmc_url(address: &str, default_port: u16) -> Result<Url, Box<dyn Error>> {
    if address.starts_with("http://") || address.starts_with("https://") {
        return Ok(address.parse()?);
    }
    Ok(format!("https://{address}:{default_port}").parse()?)
}

#[cfg(test)]
mod tests {
    use std::convert::Infallible;

    use super::*;
    use nv_redfish_bmc_mock::Expect;

    #[test]
    fn redfish_source_records_constructor_options() {
        let boot_mac = "00:11:22:33:44:55".parse().expect("mac");
        let source = RedfishSource::new(8443, true, Some(boot_mac));

        assert_eq!(source.port, 8443);
        assert!(source.insecure_tls);
        assert_eq!(source._boot_mac, Some(boot_mac));
    }

    #[test]
    fn bmc_url_adds_https_and_default_port_to_bare_address() {
        let url = bmc_url("10.0.0.1", 443).expect("url");

        assert_eq!(url.as_str(), "https://10.0.0.1/");
        assert_eq!(url.port_or_known_default(), Some(443));
    }

    #[test]
    fn bmc_url_preserves_explicit_https_url() {
        let url = bmc_url("https://bmc.example.com:8443", 443).expect("url");

        assert_eq!(url.as_str(), "https://bmc.example.com:8443/");
    }

    #[test]
    fn bmc_url_preserves_explicit_http_url() {
        let url = bmc_url("http://bmc.example.com", 443).expect("url");

        assert_eq!(url.as_str(), "http://bmc.example.com/");
    }

    #[test]
    fn bmc_url_rejects_invalid_explicit_url() {
        let error = bmc_url("https://", 443).expect_err("invalid url");

        assert!(!error.to_string().is_empty());
    }

    #[tokio::test]
    async fn collect_report_from_mock_bmc_traverses_inventory() {
        type MockBmc = nv_redfish_bmc_mock::Bmc<Infallible>;

        let bmc = Arc::new(MockBmc::default());
        bmc.expect(Expect::get("/redfish/v1", SERVICE_ROOT));
        bmc.expect(Expect::get("/redfish/v1", SERVICE_ROOT));
        bmc.expect(Expect::get("/redfish/v1/Managers", MANAGERS_COLLECTION));
        bmc.expect(Expect::get("/redfish/v1/Managers/BMC_0", MANAGER));
        bmc.expect(Expect::get("/redfish/v1/Systems", SYSTEMS_COLLECTION));
        bmc.expect(Expect::get("/redfish/v1/Systems/System", SYSTEM));
        bmc.expect(Expect::get("/redfish/v1/Chassis", CHASSIS_COLLECTION));
        bmc.expect(Expect::get("/redfish/v1/Chassis/BMC_0", CHASSIS));
        bmc.expect(Expect::get("/redfish/v1/UpdateService", UPDATE_SERVICE));
        bmc.expect(Expect::get(
            "/redfish/v1/UpdateService/FirmwareInventory",
            FIRMWARE_COLLECTION,
        ));
        bmc.expect(Expect::get(
            "/redfish/v1/UpdateService/FirmwareInventory/BMC",
            FIRMWARE,
        ));

        let raw_root = bmc
            .get::<ServiceRootSchema>(&ODataId::service_root())
            .await
            .expect("raw service root");
        let root = ServiceRoot::new(Arc::clone(&bmc))
            .await
            .expect("service root wrapper");

        let report = collect_report_from_root(&root, &raw_root)
            .await
            .expect("report");

        assert_eq!(
            report.service_root.redfish_version.as_deref(),
            Some("1.15.1")
        );
        assert_eq!(report.service_root.uuid.as_deref(), Some(SERVICE_UUID));
        assert_eq!(report.service_root.vendor.as_deref(), Some("NVIDIA"));
        assert_eq!(report.service_root.product.as_deref(), Some("Mock BMC"));

        assert_eq!(report.managers.len(), 1);
        assert_eq!(report.managers[0].id, "BMC_0");
        assert_eq!(report.managers[0].manager_type.as_deref(), Some("bmc"));
        assert_eq!(
            report.managers[0].firmware_version.as_deref(),
            Some("3.19.00")
        );

        assert_eq!(report.systems.len(), 1);
        assert_eq!(report.systems[0].id, "System");
        assert_eq!(report.systems[0].uuid.as_deref(), Some(SYSTEM_UUID));
        assert_eq!(report.systems[0].manufacturer.as_deref(), Some("NVIDIA"));
        assert_eq!(report.systems[0].model.as_deref(), Some("GB200"));
        assert_eq!(report.systems[0].serial_number.as_deref(), Some("SN123"));
        assert_eq!(report.systems[0].bios_version.as_deref(), Some("3A21"));
        assert_eq!(report.systems[0].power_state.as_deref(), Some("on"));
        assert_eq!(report.systems[0].cpu_count, Some(1));
        assert_eq!(report.systems[0].cpu_core_count, Some(72));
        assert_eq!(report.systems[0].cpu_model.as_deref(), Some("Grace"));
        assert_eq!(report.systems[0].memory_gib, Some(480.0));

        assert_eq!(report.chassis.len(), 1);
        assert_eq!(report.chassis[0].id, "BMC_0");
        assert_eq!(report.chassis[0].chassis_type.as_deref(), Some("module"));
        assert_eq!(report.chassis[0].manufacturer.as_deref(), Some("NVIDIA"));
        assert_eq!(report.chassis[0].model.as_deref(), Some("BMC"));
        assert_eq!(report.chassis[0].serial_number.as_deref(), Some("CH123"));
        assert_eq!(report.chassis[0].part_number.as_deref(), Some("PN123"));

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
        "Vendor": "NVIDIA",
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
        "Id": "System",
        "Name": "System",
        "UUID": "9AEE71E4-6E80-11EE-A8AC-74D4DD550187",
        "Manufacturer": "NVIDIA",
        "Model": "GB200",
        "SerialNumber": "SN123",
        "BiosVersion": "3A21",
        "PowerState": "On",
        "ProcessorSummary": {
            "Count": 1,
            "CoreCount": 72,
            "Model": "Grace"
        },
        "MemorySummary": {
            "TotalSystemMemoryGiB": 480
        }
    }"##;

    const CHASSIS_COLLECTION: &str = r##"{
        "@odata.id": "/redfish/v1/Chassis",
        "@odata.type": "#ChassisCollection.ChassisCollection",
        "Id": "Chassis",
        "Name": "Chassis",
        "Members@odata.count": 1,
        "Members": [{"@odata.id": "/redfish/v1/Chassis/BMC_0"}]
    }"##;

    const CHASSIS: &str = r##"{
        "@odata.id": "/redfish/v1/Chassis/BMC_0",
        "@odata.type": "#Chassis.v1_25_0.Chassis",
        "Id": "BMC_0",
        "Name": "BMC Chassis",
        "ChassisType": "Module",
        "Manufacturer": "NVIDIA",
        "Model": "BMC",
        "SerialNumber": "CH123",
        "PartNumber": "PN123"
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
}
