mod backend;
mod json;
mod table;

use std::error::Error;

use crate::models::BatchAttestation;

pub use backend::{BackendClient, BackendHttpError, BackendSink, HttpBackendClient, NonceResponse};
pub use json::JsonSink;
pub use table::TableSink;

pub trait AttestationSink {
    fn write(&self, batch: &BatchAttestation) -> Result<(), Box<dyn Error>>;
}
