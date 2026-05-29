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
        let root = &root_session.root;
        let raw_root = &root_session.raw;

        let service_root = ServiceRootData {
            redfish_version: raw_root.redfish_version.clone(),
            uuid: raw_root.uuid.flatten().map(|uuid| uuid.to_string()),
            vendor: raw_root.vendor.clone().flatten(),
            product: raw_root.product.clone().flatten(),
        };

        let managers = collect_managers(&root).await?;
        let systems = collect_systems(&root).await?;
        let chassis = collect_chassis(&root).await?;
        let firmware = collect_firmware(&root).await?;

        Ok(RedfishReport {
            service_root,
            managers,
            systems,
            chassis,
            firmware,
        })
    }
}

#[async_trait]
impl InventorySource for RedfishSource {
    async fn collect(&self, target: Target) -> SourceRecord {
        let result = self.collect_report(&target).await;
        SourceRecord { target, result }
    }
}

async fn collect_managers(root: &ServiceRoot<RedfishBmc>) -> Result<Vec<ManagerData>, String> {
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

async fn collect_systems(root: &ServiceRoot<RedfishBmc>) -> Result<Vec<SystemData>, String> {
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

async fn collect_chassis(root: &ServiceRoot<RedfishBmc>) -> Result<Vec<ChassisData>, String> {
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

async fn collect_firmware(root: &ServiceRoot<RedfishBmc>) -> Result<Vec<FirmwareData>, String> {
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
