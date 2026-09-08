use std::collections::BTreeMap;
use std::future::Future;
use std::time::Duration;

use bmc_inventory::config::{NmxcConfig, Rack};
use libnmxc::nmxc_model::{
    self, ControlPlaneState, DomainProperties as NmxcDomainProperties,
    DomainStateInfo as NmxcDomainStateInfo, GetPartitionInfoListRequest,
    GetPartitionInfoListResponse, GpuHealth, LogicalLinkState, PartitionHealth, PartitionType,
    PhysicalPortState, PortTopoInfo, PortType, ResiliencyMode, SwitchHealth, device_topo_info,
};
use libnmxc::{Endpoint, Nmxc, NmxcClientPool, NmxcError, NmxcTlsConfig};
use time::OffsetDateTime;
use time::format_description::well_known::Rfc3339;
use uuid::Uuid;

use crate::models::{
    CollectionError, DomainProperties, DomainState, GpuLocation, NvlinkDomainInventory, Partition,
    PartitionState, TopologyDevice, TopologyPort,
};

pub const GATEWAY_ID: &str = "fleet-intelligence-oob-collector";

pub async fn collect(racks: &[Rack], timeout: Duration) -> Vec<NvlinkDomainInventory> {
    let mut inventories = Vec::new();
    for rack in racks {
        if let Some(nmxc) = &rack.nmxc {
            inventories.push(collect_rack(rack, nmxc, timeout).await);
        }
    }
    inventories
}

pub async fn discover_domain_uuids(
    racks: &[Rack],
    timeout: Duration,
) -> (BTreeMap<String, String>, Vec<CollectionError>) {
    let mut domains = BTreeMap::new();
    let mut errors = Vec::new();
    for rack in racks {
        let Some(config) = &rack.nmxc else {
            continue;
        };
        let mut inventory = empty_inventory(rack.id.clone(), config.endpoint_url());
        let mut client = match connect(config, timeout).await {
            Ok(client) => client,
            Err((operation, error)) => {
                push_error(&mut inventory, operation, error);
                errors.extend(inventory.errors);
                continue;
            }
        };
        match rpc(timeout, "Hello", client.hello(GATEWAY_ID)).await {
            Ok(hello) => map_hello(&mut inventory, hello),
            Err(error) => push_error(&mut inventory, "Hello", error),
        }
        match inventory.domain_uuid {
            Some(domain_uuid) => {
                domains.insert(rack.id.clone(), domain_uuid);
            }
            None => errors.extend(inventory.errors),
        }
    }
    (domains, errors)
}

async fn collect_rack(
    rack: &Rack,
    config: &NmxcConfig,
    timeout: Duration,
) -> NvlinkDomainInventory {
    let endpoint = config.endpoint_url();
    let mut inventory = empty_inventory(rack.id.clone(), endpoint.clone());
    let mut client = match connect(config, timeout).await {
        Ok(client) => client,
        Err((operation, error)) => {
            push_error(&mut inventory, operation, error);
            return inventory;
        }
    };

    match rpc(timeout, "Hello", client.hello(GATEWAY_ID)).await {
        Ok(hello) => map_hello(&mut inventory, hello),
        Err(error) => {
            push_error(&mut inventory, "Hello", error);
            return inventory;
        }
    }
    collect_remaining(&mut inventory, client.as_mut(), timeout).await;
    inventory
}

async fn connect(
    config: &NmxcConfig,
    timeout: Duration,
) -> Result<Box<dyn Nmxc>, (&'static str, String)> {
    let mut builder = NmxcClientPool::builder().timeout(timeout);
    if config.uses_tls() {
        builder = builder.tls(NmxcTlsConfig {
            ca_cert_path: config.ca_file.clone(),
            client_cert_path: config.client_cert_file.clone(),
            client_key_path: config.client_key_file.clone(),
            authority: config.server_name.clone(),
        });
    }
    let pool = match builder.build() {
        Ok(pool) => pool,
        Err(error) => return Err(("configure", error.to_string())),
    };
    let endpoint = match Endpoint::new(config.endpoint_url()) {
        Ok(endpoint) => endpoint,
        Err(error) => return Err(("endpoint", error.to_string())),
    };
    pool.create_client(endpoint)
        .await
        .map_err(|error| ("connect", error.to_string()))
}

async fn collect_remaining(
    inventory: &mut NvlinkDomainInventory,
    client: &mut dyn Nmxc,
    timeout: Duration,
) {
    match rpc(
        timeout,
        "GetDomainStateInfo",
        client.get_domain_state_info(None, GATEWAY_ID),
    )
    .await
    {
        Ok(response)
            if validate_response_domain(
                inventory,
                "GetDomainStateInfo",
                response.server_header.as_ref(),
            ) =>
        {
            inventory.state = Some(map_domain_state(response));
        }
        Ok(_) => {}
        Err(error) => push_error(inventory, "GetDomainStateInfo", error),
    }
    match rpc(
        timeout,
        "GetDomainProperties",
        client.get_domain_properties(None, GATEWAY_ID),
    )
    .await
    {
        Ok(response)
            if validate_response_domain(
                inventory,
                "GetDomainProperties",
                response.server_header.as_ref(),
            ) =>
        {
            inventory.properties = Some(map_domain_properties(response));
        }
        Ok(_) => {}
        Err(error) => push_error(inventory, "GetDomainProperties", error),
    }
    match rpc(
        timeout,
        "GetPartitionInfoList",
        client.get_partition_info_list(GetPartitionInfoListRequest {
            context: None,
            partition_id_list: Vec::new(),
            partition_name_list: Vec::new(),
            gateway_id: GATEWAY_ID.to_string(),
        }),
    )
    .await
    {
        Ok(response)
            if validate_response_domain(
                inventory,
                "GetPartitionInfoList",
                response.server_header.as_ref(),
            ) =>
        {
            for partition in &response.partition_info_list {
                if matches!(
                    map_partition_state(PartitionHealth::try_from(partition.health).ok()),
                    PartitionState::Unknown
                ) {
                    let partition_id = partition
                        .partition_id
                        .as_ref()
                        .map(|id| id.partition_id.to_string())
                        .unwrap_or_else(|| "unknown".to_string());
                    push_error(
                        inventory,
                        "GetPartitionInfoList",
                        format!(
                            "partition {partition_id} has unsupported health value {}",
                            partition.health
                        ),
                    );
                }
            }
            inventory.partitions = map_partitions(response);
        }
        Ok(_) => {}
        Err(error) => push_error(inventory, "GetPartitionInfoList", error),
    }
    match rpc(
        timeout,
        "GetTopologyInfo",
        client.get_topology_info(None, GATEWAY_ID),
    )
    .await
    {
        Ok(response)
            if validate_response_domain(
                inventory,
                "GetTopologyInfo",
                response.server_header.as_ref(),
            ) =>
        {
            inventory.topology = map_topology(response);
        }
        Ok(_) => {}
        Err(error) => push_error(inventory, "GetTopologyInfo", error),
    }
}

async fn rpc<T>(
    timeout: Duration,
    operation: &'static str,
    future: impl Future<Output = Result<T, NmxcError>>,
) -> Result<T, String> {
    tokio::time::timeout(timeout, future)
        .await
        .map_err(|_| format!("{operation} timed out after {} seconds", timeout.as_secs()))?
        .map_err(|error| error.to_string())
}

fn empty_inventory(rack_id: String, endpoint: String) -> NvlinkDomainInventory {
    NvlinkDomainInventory {
        schema_version: "v1alpha1",
        collected_at: OffsetDateTime::now_utc()
            .format(&Rfc3339)
            .unwrap_or_else(|_| "unknown".to_string()),
        rack_id,
        endpoint,
        domain_uuid: None,
        nmxc_version: None,
        capabilities: Vec::new(),
        state: None,
        properties: None,
        partitions: Vec::new(),
        topology: Vec::new(),
        errors: Vec::new(),
    }
}

fn push_error(
    inventory: &mut NvlinkDomainInventory,
    operation: &str,
    error: impl std::fmt::Display,
) {
    inventory.errors.push(CollectionError {
        rack_id: inventory.rack_id.clone(),
        operation: operation.to_string(),
        message: error.to_string(),
    });
}

fn map_hello(inventory: &mut NvlinkDomainInventory, hello: nmxc_model::ServerHello) {
    if let Some(header) = hello.server_header {
        match Uuid::parse_str(&header.domain_uuid) {
            Ok(domain_uuid) => inventory.domain_uuid = Some(domain_uuid.to_string()),
            Err(error) => push_error(
                inventory,
                "Hello",
                format!("invalid domain UUID {:?}: {error}", header.domain_uuid),
            ),
        }
        if !header.app_ver.trim().is_empty() {
            inventory.nmxc_version = Some(header.app_ver);
        }
    } else {
        push_error(inventory, "Hello", "response missing server_header");
    }
    inventory.capabilities = hello.capabilities;
}

fn validate_response_domain(
    inventory: &mut NvlinkDomainInventory,
    operation: &str,
    header: Option<&nmxc_model::ServerHeader>,
) -> bool {
    let Some(header) = header else {
        push_error(inventory, operation, "response missing server_header");
        return false;
    };
    let Some(expected) = inventory.domain_uuid.as_deref() else {
        push_error(
            inventory,
            operation,
            "Hello did not establish a domain UUID",
        );
        return false;
    };
    match Uuid::parse_str(&header.domain_uuid) {
        Ok(actual) if actual.to_string() == expected => true,
        Ok(actual) => {
            push_error(
                inventory,
                operation,
                format!("domain UUID {actual} does not match Hello domain UUID {expected}"),
            );
            false
        }
        Err(error) => {
            push_error(
                inventory,
                operation,
                format!("invalid domain UUID {:?}: {error}", header.domain_uuid),
            );
            false
        }
    }
}

fn map_domain_state(state: NmxcDomainStateInfo) -> DomainState {
    DomainState {
        control_plane_state: enum_name(
            ControlPlaneState::try_from(state.control_plane_state).ok(),
            state.control_plane_state,
        ),
        controller_health: enum_name(
            nmxc_model::NmxControllerHealth::try_from(state.nmx_controller_health).ok(),
            state.nmx_controller_health,
        ),
        config_status_description: state.config_status_description,
        available_multicast_groups: state.available_multicast_groups,
    }
}

fn map_domain_properties(properties: NmxcDomainProperties) -> DomainProperties {
    DomainProperties {
        max_compute_nodes: properties.max_compute_nodes,
        max_gpus_per_compute_node: properties.max_gpus_per_compute_node,
        max_gpu_nv_links: properties.max_gpu_nv_links,
        max_switch_nodes: properties.max_switch_nodes,
        max_switches_per_switch_node: properties.max_switches_per_switch_node,
        max_switch_nv_links: properties.max_switch_nv_links,
        max_num_partitions: properties.max_num_partitions,
        max_num_ports: properties.max_num_ports,
        line_rate_mbps: properties.line_rate_mbps,
    }
}

fn map_partitions(response: GetPartitionInfoListResponse) -> Vec<Partition> {
    response
        .partition_info_list
        .into_iter()
        .map(|partition| {
            let health = PartitionHealth::try_from(partition.health).ok();
            let (resiliency_mode, multicast_groups_limit) = partition
                .attr
                .map(|attr| {
                    (
                        Some(enum_name(
                            ResiliencyMode::try_from(attr.resiliency_mode).ok(),
                            attr.resiliency_mode,
                        )),
                        Some(attr.multicast_groups_limit),
                    )
                })
                .unwrap_or((None, None));
            Partition {
                partition_id: partition.partition_id.map(|id| id.partition_id),
                name: partition.name,
                state: map_partition_state(health),
                raw_health: enum_name(health, partition.health),
                partition_type: enum_name(
                    PartitionType::try_from(partition.partition_type).ok(),
                    partition.partition_type,
                ),
                resiliency_mode,
                multicast_groups_limit,
                num_allocated_multicast_groups: partition.num_allocated_multicast_groups,
                num_gpus: partition.num_gpus,
                gpu_uids: partition.gpu_uid_list,
                gpu_locations: partition
                    .gpu_location_list
                    .into_iter()
                    .map(|gpu| {
                        let location = gpu.loc.unwrap_or_default();
                        GpuLocation {
                            chassis_id: location.chassis_id,
                            slot_id: location.slot_id,
                            host_id: location.host_id,
                            gpu_id: gpu.gpu_id,
                        }
                    })
                    .collect(),
            }
        })
        .collect()
}

fn map_partition_state(health: Option<PartitionHealth>) -> PartitionState {
    match health {
        Some(PartitionHealth::NmxPartitionHealthHealthy) => PartitionState::Active,
        Some(PartitionHealth::NmxPartitionHealthDegradedBandwidth)
        | Some(PartitionHealth::NmxPartitionHealthDegraded) => PartitionState::Degraded,
        Some(PartitionHealth::NmxPartitionHealthUnhealthy) => PartitionState::Failed,
        _ => PartitionState::Unknown,
    }
}

fn map_topology(topology: nmxc_model::FmTopologyInfo) -> Vec<TopologyDevice> {
    topology
        .device_topo_info
        .into_iter()
        .filter_map(|device| match device.device? {
            device_topo_info::Device::SwitchTopoInfo(info) => {
                let location = info.loc.unwrap_or_default();
                let position = location.location;
                Some(TopologyDevice {
                    device_type: "NVSWITCH".to_string(),
                    device_uid: info.device_uid,
                    topology_id: info.topology_id,
                    chassis_serial_number: location.chassis_serial_number,
                    tray_index: location.tray_index,
                    chassis_id: position.as_ref().map(|value| value.chassis_id),
                    slot_id: position.as_ref().map(|value| value.slot_id),
                    host_id: position.as_ref().map(|value| value.host_id),
                    description: info.description,
                    health: enum_name(
                        SwitchHealth::try_from(info.device_health).ok(),
                        info.device_health,
                    ),
                    partition_ids: info
                        .partition_id
                        .into_iter()
                        .map(|id| id.partition_id)
                        .collect(),
                    ports: info.port_topo_info.into_iter().map(map_port).collect(),
                })
            }
            device_topo_info::Device::GpuTopoInfo(info) => {
                let location = info.loc.unwrap_or_default();
                let position = location.location;
                Some(TopologyDevice {
                    device_type: "GPU".to_string(),
                    device_uid: info.device_uid,
                    topology_id: info.topology_id,
                    chassis_serial_number: location.chassis_serial_number,
                    tray_index: location.tray_index,
                    chassis_id: position.as_ref().map(|value| value.chassis_id),
                    slot_id: position.as_ref().map(|value| value.slot_id),
                    host_id: position.as_ref().map(|value| value.host_id),
                    description: info.description,
                    health: enum_name(
                        GpuHealth::try_from(info.device_health).ok(),
                        info.device_health,
                    ),
                    partition_ids: info
                        .partition_id
                        .into_iter()
                        .map(|id| id.partition_id)
                        .collect(),
                    ports: info.port_topo_info.into_iter().map(map_port).collect(),
                })
            }
        })
        .collect()
}

fn map_port(port: PortTopoInfo) -> TopologyPort {
    TopologyPort {
        port_num: port.port_num,
        port_uid: port.port_uid,
        port_type: enum_name(PortType::try_from(port.port_type).ok(), port.port_type),
        peer_device_uid: port.peer_port_device_uid,
        peer_port_num: port.peer_port_num,
        physical_state: enum_name(
            PhysicalPortState::try_from(port.physical_state).ok(),
            port.physical_state,
        ),
        logical_state: enum_name(
            LogicalLinkState::try_from(port.logical_state).ok(),
            port.logical_state,
        ),
        partition_ids: port
            .partition_id_list
            .into_iter()
            .map(|id| id.partition_id)
            .collect(),
    }
}

trait EnumName {
    fn enum_name(&self) -> &'static str;
}

macro_rules! impl_enum_name {
    ($($kind:ty),+ $(,)?) => {
        $(impl EnumName for $kind {
            fn enum_name(&self) -> &'static str {
                self.as_str_name()
            }
        })+
    };
}

impl_enum_name!(
    ControlPlaneState,
    nmxc_model::NmxControllerHealth,
    PartitionHealth,
    PartitionType,
    SwitchHealth,
    GpuHealth,
    PortType,
    PhysicalPortState,
    LogicalLinkState,
    ResiliencyMode,
);

fn enum_name<T: EnumName>(value: Option<T>, raw: i32) -> String {
    value
        .map(|value| value.enum_name().to_string())
        .unwrap_or_else(|| format!("UNKNOWN({raw})"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn maps_partition_health_to_prd_state() {
        assert!(matches!(
            map_partition_state(Some(PartitionHealth::NmxPartitionHealthHealthy)),
            PartitionState::Active
        ));
        assert!(matches!(
            map_partition_state(Some(PartitionHealth::NmxPartitionHealthDegradedBandwidth)),
            PartitionState::Degraded
        ));
        assert!(matches!(
            map_partition_state(Some(PartitionHealth::NmxPartitionHealthUnhealthy)),
            PartitionState::Failed
        ));
        assert!(matches!(map_partition_state(None), PartitionState::Unknown));
    }

    #[test]
    fn invalid_domain_uuid_is_a_scoped_collection_error() {
        let mut inventory = empty_inventory("rack-1".to_string(), "http://nmxc".to_string());
        map_hello(
            &mut inventory,
            nmxc_model::ServerHello {
                server_header: Some(nmxc_model::ServerHeader {
                    domain_uuid: "not-a-uuid".to_string(),
                    app_ver: "1.2.3".to_string(),
                    ..Default::default()
                }),
                ..Default::default()
            },
        );
        assert!(inventory.domain_uuid.is_none());
        assert_eq!(inventory.nmxc_version.as_deref(), Some("1.2.3"));
        assert_eq!(inventory.errors.len(), 1);
        assert_eq!(inventory.errors[0].operation, "Hello");
    }

    #[test]
    fn rejects_a_response_from_a_different_domain() {
        let mut inventory = empty_inventory("rack-1".to_string(), "http://nmxc".to_string());
        inventory.domain_uuid = Some("11111111-1111-4111-8111-111111111111".to_string());
        let header = nmxc_model::ServerHeader {
            domain_uuid: "c2a9196f-9264-4c42-8298-09d2d1dc14d5".to_string(),
            ..Default::default()
        };

        assert!(!validate_response_domain(
            &mut inventory,
            "GetTopologyInfo",
            Some(&header)
        ));
        assert_eq!(inventory.errors.len(), 1);
        assert!(inventory.errors[0].message.contains("does not match"));
    }

    #[test]
    fn local_sinks_render_success_and_failure_fields() {
        let mut inventory = empty_inventory("rack-1".to_string(), "http://nmxc".to_string());
        inventory.domain_uuid = Some("11111111-1111-4111-8111-111111111111".to_string());
        inventory.partitions.push(Partition {
            partition_id: Some(32766),
            name: "Default Partition".to_string(),
            state: PartitionState::Active,
            raw_health: "NMX_PARTITION_HEALTH_HEALTHY".to_string(),
            partition_type: "NMX_PARTITION_TYPE_LOCATION_BASED".to_string(),
            resiliency_mode: Some("NMX_RESILIENCY_MODE_FULL_BANDWIDTH".to_string()),
            multicast_groups_limit: Some(16),
            num_allocated_multicast_groups: 1,
            num_gpus: 8,
            gpu_uids: vec![101],
            gpu_locations: Vec::new(),
        });
        push_error(&mut inventory, "GetTopologyInfo", "unsupported");

        let json = crate::sink::render_json(&[inventory.clone()]).expect("JSON output");
        let table = crate::sink::render_table(&[inventory], true);

        assert!(json.contains(r#""domainUuid": "11111111-1111-4111-8111-111111111111""#));
        assert!(json.contains(r#""operation": "GetTopologyInfo""#));
        assert!(json.contains(r#""state": "ACTIVE""#));
        assert!(json.contains(r#""multicastGroupsLimit": 16"#));
        assert!(table.contains("rack-1"));
        assert!(table.contains("ACTIVE"));
        assert!(table.contains("GetTopologyInfo"));
        assert!(table.contains("unsupported"));
    }

    #[tokio::test]
    async fn rpc_deadline_bounds_an_unresponsive_controller() {
        let error = rpc(
            Duration::from_millis(1),
            "GetTopologyInfo",
            std::future::pending::<Result<(), NmxcError>>(),
        )
        .await
        .expect_err("pending RPC must time out");

        assert!(error.contains("GetTopologyInfo timed out"));
    }
}
