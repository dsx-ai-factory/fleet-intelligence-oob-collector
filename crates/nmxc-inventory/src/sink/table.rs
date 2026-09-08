use std::fmt::Write;

use crate::models::NvlinkDomainInventory;

pub fn render_table(inventories: &[NvlinkDomainInventory], detail: bool) -> String {
    let mut output = String::new();
    writeln!(
        output,
        "{:<18} {:<38} {:<18} {:>10} {:>10} {:>8}",
        "RACK", "NVLINK DOMAIN UUID", "NMX-C VERSION", "PARTITIONS", "DEVICES", "ERRORS"
    )
    .expect("write to string");

    for inventory in inventories {
        writeln!(
            output,
            "{:<18} {:<38} {:<18} {:>10} {:>10} {:>8}",
            inventory.rack_id,
            inventory.domain_uuid.as_deref().unwrap_or("-"),
            inventory.nmxc_version.as_deref().unwrap_or("-"),
            inventory.partitions.len(),
            inventory.topology.len(),
            inventory.errors.len(),
        )
        .expect("write to string");

        if detail {
            for partition in &inventory.partitions {
                writeln!(
                    output,
                    "  partition {:<8} {:<24} {:<10} gpus={}",
                    partition
                        .partition_id
                        .map(|id| id.to_string())
                        .unwrap_or_else(|| "-".to_string()),
                    partition.name,
                    partition.state,
                    partition.num_gpus,
                )
                .expect("write to string");
            }
            for error in &inventory.errors {
                writeln!(output, "  error {:<24} {}", error.operation, error.message)
                    .expect("write to string");
            }
        }
    }
    output
}
