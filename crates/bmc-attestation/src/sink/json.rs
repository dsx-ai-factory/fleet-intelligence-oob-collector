use std::error::Error;

use crate::models::BatchAttestation;

use super::AttestationSink;

#[derive(Default)]
pub struct JsonSink;

impl AttestationSink for JsonSink {
    fn write(&self, batch: &BatchAttestation) -> Result<(), Box<dyn Error>> {
        println!("{}", render_json(batch)?);
        Ok(())
    }
}

fn render_json(batch: &BatchAttestation) -> Result<String, serde_json::Error> {
    serde_json::to_string_pretty(batch)
}

#[cfg(test)]
mod tests {
    use crate::models::{BatchAttestation, NodeAttestation};

    use super::*;

    #[test]
    fn json_sink_renders_node_uuid_keys() {
        let rendered = render_json(&BatchAttestation::new(
            "2026-01-01T00:00:00Z".to_string(),
            vec![NodeAttestation::failed(
                "node-1".to_string(),
                "node-a".to_string(),
                "10.0.0.1".to_string(),
                "timeout",
            )],
        ))
        .expect("json");

        assert!(rendered.contains(r#""resultsByNodeUuid""#));
        assert!(rendered.contains(r#""node-1""#));
        assert!(rendered.contains(r#""success": false"#));
    }
}
