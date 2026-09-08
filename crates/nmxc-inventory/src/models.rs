use serde::Serialize;

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct NvlinkDomainInventory {
    pub schema_version: &'static str,
    pub collected_at: String,
    pub rack_id: String,
    pub endpoint: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub domain_uuid: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub nmxc_version: Option<String>,
    pub capabilities: Vec<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub state: Option<DomainState>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub properties: Option<DomainProperties>,
    pub partitions: Vec<Partition>,
    pub topology: Vec<TopologyDevice>,
    pub errors: Vec<CollectionError>,
}

impl NvlinkDomainInventory {
    pub fn failed(&self) -> bool {
        self.domain_uuid.is_none()
    }
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CollectionError {
    pub rack_id: String,
    pub operation: String,
    pub message: String,
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DomainState {
    pub control_plane_state: String,
    pub controller_health: String,
    pub config_status_description: String,
    pub available_multicast_groups: u32,
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DomainProperties {
    pub max_compute_nodes: u32,
    pub max_gpus_per_compute_node: u32,
    pub max_gpu_nv_links: u32,
    pub max_switch_nodes: u32,
    pub max_switches_per_switch_node: u32,
    pub max_switch_nv_links: u32,
    pub max_num_partitions: u32,
    pub max_num_ports: u32,
    pub line_rate_mbps: u32,
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Partition {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub partition_id: Option<u32>,
    pub name: String,
    pub state: PartitionState,
    pub raw_health: String,
    pub partition_type: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub resiliency_mode: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub multicast_groups_limit: Option<u32>,
    pub num_allocated_multicast_groups: u32,
    pub num_gpus: u32,
    pub gpu_uids: Vec<u64>,
    pub gpu_locations: Vec<GpuLocation>,
}

#[derive(Clone, Copy, Debug, Serialize)]
#[serde(rename_all = "UPPERCASE")]
pub enum PartitionState {
    Active,
    Degraded,
    Failed,
    Unknown,
}

impl std::fmt::Display for PartitionState {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let value = match self {
            Self::Active => "ACTIVE",
            Self::Degraded => "DEGRADED",
            Self::Failed => "FAILED",
            Self::Unknown => "UNKNOWN",
        };
        formatter.write_str(value)
    }
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct GpuLocation {
    pub chassis_id: u64,
    pub slot_id: u64,
    pub host_id: u64,
    pub gpu_id: u32,
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct TopologyDevice {
    pub device_type: String,
    pub device_uid: u64,
    pub topology_id: u64,
    pub chassis_serial_number: String,
    pub tray_index: u64,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub chassis_id: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub slot_id: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub host_id: Option<u64>,
    pub description: String,
    pub health: String,
    pub partition_ids: Vec<u32>,
    pub ports: Vec<TopologyPort>,
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct TopologyPort {
    pub port_num: u64,
    pub port_uid: u64,
    pub port_type: String,
    pub peer_device_uid: u64,
    pub peer_port_num: u64,
    pub physical_state: String,
    pub logical_state: String,
    pub partition_ids: Vec<u32>,
}
