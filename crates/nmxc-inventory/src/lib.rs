mod collector;
pub mod models;
pub mod sink;

pub use collector::{GATEWAY_ID, collect, discover_domain_uuids};
