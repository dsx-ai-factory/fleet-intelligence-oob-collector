use std::error::Error;

use crate::inventory::types::NodeInventory;

use super::InventorySink;

pub struct BackendSink<C = HttpBackendClient> {
    client: C,
}

impl BackendSink<HttpBackendClient> {
    pub fn new(endpoint: impl Into<String>) -> Self {
        Self::with_client(HttpBackendClient::new(endpoint))
    }
}

impl<C> BackendSink<C> {
    pub fn with_client(client: C) -> Self {
        Self { client }
    }
}

impl<C> InventorySink for BackendSink<C>
where
    C: BackendClient,
{
    fn write(&self, inventories: &[NodeInventory]) -> Result<(), Box<dyn Error>> {
        self.client.put_inventory(inventories)
    }
}

pub trait BackendClient {
    fn put_inventory(&self, inventories: &[NodeInventory]) -> Result<(), Box<dyn Error>>;
}

pub struct HttpBackendClient {
    endpoint: String,
    client: reqwest::blocking::Client,
}

impl HttpBackendClient {
    pub fn new(endpoint: impl Into<String>) -> Self {
        Self {
            endpoint: endpoint.into(),
            client: reqwest::blocking::Client::new(),
        }
    }
}

impl BackendClient for HttpBackendClient {
    fn put_inventory(&self, inventories: &[NodeInventory]) -> Result<(), Box<dyn Error>> {
        self.client
            .put(&self.endpoint)
            .json(inventories)
            .send()?
            .error_for_status()?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use std::cell::RefCell;
    use std::rc::Rc;

    use super::*;

    #[test]
    fn backend_sink_puts_node_inventory_batch() {
        let put_batches = Rc::new(RefCell::new(Vec::new()));
        let sink = BackendSink::with_client(RecordingClient {
            put_batches: Rc::clone(&put_batches),
        });

        sink.write(&[inventory("node-a"), inventory("node-b")])
            .expect("write");

        let put_batches = put_batches.borrow();
        assert_eq!(put_batches.len(), 1);
        assert_eq!(put_batches[0].len(), 2);
        assert_eq!(put_batches[0][0], "node-a");
        assert_eq!(put_batches[0][1], "node-b");
    }

    #[derive(Clone)]
    struct RecordingClient {
        put_batches: Rc<RefCell<Vec<Vec<String>>>>,
    }

    impl BackendClient for RecordingClient {
        fn put_inventory(&self, inventories: &[NodeInventory]) -> Result<(), Box<dyn Error>> {
            self.put_batches.borrow_mut().push(
                inventories
                    .iter()
                    .map(|inventory| inventory.source.target.clone())
                    .collect(),
            );
            Ok(())
        }
    }

    fn inventory(target: &str) -> NodeInventory {
        NodeInventory {
            schema_version: crate::inventory::types::INVENTORY_SCHEMA_VERSION.to_string(),
            collected_at: "2026-01-01T00:00:00Z".to_string(),
            source: crate::inventory::types::InventorySourceInfo {
                source_type: "redfish".to_string(),
                target: target.to_string(),
                address: "10.0.0.1".to_string(),
                redfish_version: None,
                service_uuid: None,
                vendor: None,
            },
            node: None,
            manager: None,
            chassis: Vec::new(),
            devices: Vec::new(),
            firmware: Vec::new(),
            errors: Vec::new(),
            collection_error: None,
        }
    }
}
