mod nico;
mod targets;

pub use crate::models::TargetKind;
pub use nico::{NicoConfigOptions, render_nico_config};
pub use targets::{CollectorConfig, Target, load_config, load_targets};
