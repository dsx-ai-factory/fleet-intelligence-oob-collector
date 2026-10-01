pub mod config;
pub mod models;
pub mod sink;
pub mod source;

/// Collector release version shared by the CLI and inventory uploads.
pub const COLLECTOR_VERSION: &str = match option_env!("FLEETINT_BUILD_VERSION") {
    Some(version) if !version.is_empty() => version,
    _ => env!("CARGO_PKG_VERSION"),
};
