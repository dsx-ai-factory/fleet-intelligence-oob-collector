use std::error::Error;

use crate::models::NodeInventory;

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

        sink.write(&[inventory("10.0.0.1"), inventory("10.0.0.2")])
            .expect("write");

        let put_batches = put_batches.borrow();
        assert_eq!(put_batches.len(), 1);
        assert_eq!(put_batches[0].len(), 2);
        assert_eq!(put_batches[0][0], "10.0.0.1");
        assert_eq!(put_batches[0][1], "10.0.0.2");
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
                    .map(|inventory| inventory.source.address.clone())
                    .collect(),
            );
            Ok(())
        }
    }

    fn inventory(address: &str) -> NodeInventory {
        NodeInventory {
            schema_version: crate::models::INVENTORY_SCHEMA_VERSION.to_string(),
            collected_at: "2026-01-01T00:00:00Z".to_string(),
            uuid: None,
            source: crate::models::InventorySourceInfo {
                source_type: "redfish".to_string(),
                address: address.to_string(),
                redfish_version: None,
                service_uuid: None,
                vendor: None,
            },
            primary_system_id: None,
            managers: Vec::new(),
            systems: Vec::new(),
            chassis: Vec::new(),
            firmware: Vec::new(),
            domain_errors: Vec::new(),
            target_error: None,
        }
    }
}
