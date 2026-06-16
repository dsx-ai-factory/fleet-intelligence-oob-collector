use std::error::Error;
use std::fmt;

use crate::models::{BatchAttestation, NodeAttestation};

use super::AttestationSink;

#[derive(Default)]
pub struct TableSink {
    detail: bool,
}

#[derive(Debug)]
struct SummaryRow {
    node_uuid: String,
    target: String,
    address: String,
    success: String,
    evidences: String,
    component_errors: String,
    nonce_refresh: String,
    error: String,
}

#[derive(Debug)]
struct EvidenceRow {
    component: String,
    arch: String,
    vbios: String,
    version: String,
}

impl TableSink {
    pub fn new(detail: bool) -> Self {
        Self { detail }
    }
}

impl AttestationSink for TableSink {
    fn write(&self, batch: &BatchAttestation) -> Result<(), Box<dyn Error>> {
        print!("{}", render_attestation_tables(batch, self.detail));
        Ok(())
    }
}

fn render_attestation_tables(batch: &BatchAttestation, detail: bool) -> String {
    let rows = batch
        .results_by_node_uuid
        .values()
        .map(SummaryRow::from)
        .collect::<Vec<_>>();
    let mut out = render_table(
        &[
            "Node UUID",
            "Target",
            "Address",
            "Success",
            "Evidence",
            "Component Errors",
            "Nonce Refresh",
            "Error",
        ],
        rows.iter().map(SummaryRow::values),
    );

    if detail {
        for node in batch.results_by_node_uuid.values() {
            write_node_details(&mut out, node);
        }
    }
    out
}

impl From<&NodeAttestation> for SummaryRow {
    fn from(node: &NodeAttestation) -> Self {
        Self {
            node_uuid: node.node_uuid.clone(),
            target: node.target_name.clone(),
            address: node.bmc_address.clone(),
            success: node.success.to_string(),
            evidences: node.evidences.len().to_string(),
            component_errors: node.component_errors.len().to_string(),
            nonce_refresh: node.nonce_refresh_timestamp.clone().unwrap_or_default(),
            error: node.error_message.clone().unwrap_or_default(),
        }
    }
}

impl SummaryRow {
    fn values(&self) -> [String; 8] {
        [
            self.node_uuid.clone(),
            self.target.clone(),
            self.address.clone(),
            self.success.clone(),
            self.evidences.clone(),
            self.component_errors.clone(),
            self.nonce_refresh.clone(),
            self.error.clone(),
        ]
    }
}

impl EvidenceRow {
    fn from_evidence(evidence: &crate::models::AttestationEvidence) -> Self {
        Self {
            component: evidence.component_id.clone(),
            arch: evidence.arch.clone().unwrap_or_default(),
            vbios: evidence.vbios_version.clone().unwrap_or_default(),
            version: evidence.version.clone().unwrap_or_default(),
        }
    }

    fn values(&self) -> [String; 4] {
        [
            self.component.clone(),
            self.arch.clone(),
            self.vbios.clone(),
            self.version.clone(),
        ]
    }
}

fn write_node_details(out: &mut String, node: &NodeAttestation) {
    out.push('\n');
    out.push_str(&format!(
        "Node: {} ({})\n",
        node.target_name, node.node_uuid
    ));
    out.push_str("Evidence\n");
    out.push_str(&render_table(
        &["Component", "Arch", "VBIOS", "Version"],
        node.evidences
            .iter()
            .map(EvidenceRow::from_evidence)
            .map(|row| row.values()),
    ));
}

fn render_table<const N: usize>(
    headers: &[&str; N],
    rows: impl IntoIterator<Item = [String; N]>,
) -> String {
    let rows = rows.into_iter().collect::<Vec<_>>();
    let mut widths = headers.map(str::len);
    for row in &rows {
        for (idx, value) in row.iter().enumerate() {
            widths[idx] = widths[idx].max(value.len());
        }
    }

    let mut out = String::new();
    write_separator(&mut out, &widths);
    write_row(&mut out, headers.as_slice(), &widths);
    write_separator(&mut out, &widths);
    for row in &rows {
        write_row(&mut out, row, &widths);
    }
    write_separator(&mut out, &widths);
    out
}

fn write_separator(out: &mut String, widths: &[usize]) {
    out.push('+');
    for width in widths {
        out.push_str(&"-".repeat(width + 2));
        out.push('+');
    }
    out.push('\n');
}

fn write_row<T: fmt::Display>(out: &mut String, values: &[T], widths: &[usize]) {
    out.push('|');
    for (value, width) in values.iter().zip(widths) {
        out.push_str(&format!(" {value:<width$} |"));
    }
    out.push('\n');
}

#[cfg(test)]
mod tests {
    use crate::models::{BatchAttestation, NodeAttestation};

    use super::*;

    #[test]
    fn renders_summary() {
        let rendered = render_attestation_tables(
            &BatchAttestation::new(
                "2026-01-01T00:00:00Z".to_string(),
                vec![NodeAttestation::failed(
                    "node-1".to_string(),
                    "node-a".to_string(),
                    "10.0.0.1".to_string(),
                    "timeout",
                )],
            ),
            false,
        );

        assert!(rendered.contains("Node UUID"));
        assert!(rendered.contains("node-1"));
        assert!(rendered.contains("timeout"));
    }
}
