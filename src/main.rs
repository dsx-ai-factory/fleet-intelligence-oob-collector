use std::error::Error;
use std::fs::OpenOptions;
use std::io::Write;
#[cfg(unix)]
use std::os::unix::fs::{OpenOptionsExt, PermissionsExt};

use bmc_inventory::config::{self, NicoConfigOptions};
use clap::{Args, Parser, Subcommand, ValueEnum};
use fleet_intelligence_oob::{
    DEFAULT_BMC_BURST, DEFAULT_BMC_MAX_IN_FLIGHT, DEFAULT_BMC_REQUESTS_PER_SECOND,
    DEFAULT_INVENTORY_INTERVAL_SECONDS, DEFAULT_REDFISH_TIMEOUT_SECONDS,
    DEFAULT_TARGET_CONCURRENCY, InventoryFormat, InventoryOptions, RuntimeOptions, run_inventory,
    run_runtime,
};
use nmxc_inventory::sink::{render_json as render_nmxc_json, render_table as render_nmxc_table};

#[derive(Debug, Parser)]
#[command(name = "fleet-intelligence-oob")]
#[command(about = "Fleet Intelligence out-of-band collector")]
struct Cli {
    #[command(subcommand)]
    command: Command,
}

#[derive(Debug, Subcommand)]
enum Command {
    #[command(about = "Collect BMC data and write it to local output")]
    Inventory(InventoryArgs),
    #[command(about = "Collect NMX-C fabric metadata and write it to local output")]
    FabricInventory(FabricInventoryArgs),
    #[command(about = "Continuously collect BMC data and export it to Fleet Intelligence")]
    Run(RuntimeArgs),
    #[command(about = "Generate NICo static configuration from the collector config")]
    RenderNicoConfig(RenderNicoConfigArgs),
}

#[derive(Debug, Parser)]
struct FabricInventoryArgs {
    #[arg(short, long, default_value = "config.toml")]
    config: String,

    #[arg(long, default_value_t = DEFAULT_REDFISH_TIMEOUT_SECONDS)]
    timeout_seconds: u64,

    #[arg(long)]
    detail: bool,

    #[arg(long, default_value = "table", value_enum)]
    format: CliInventoryFormat,
}

#[derive(Debug, Parser)]
struct InventoryArgs {
    #[command(flatten)]
    collector: CollectorArgs,

    #[arg(long)]
    detail: bool,

    #[arg(long, default_value = "table", value_enum)]
    format: CliInventoryFormat,
}

#[derive(Clone, Debug, Args)]
struct CollectorArgs {
    #[arg(short, long, default_value = "config.toml")]
    config: String,

    #[arg(long)]
    insecure_tls: bool,

    #[arg(long, default_value_t = DEFAULT_TARGET_CONCURRENCY)]
    target_concurrency: usize,

    #[arg(long, default_value_t = DEFAULT_BMC_MAX_IN_FLIGHT)]
    bmc_max_in_flight: u32,

    #[arg(long, default_value_t = DEFAULT_BMC_REQUESTS_PER_SECOND)]
    bmc_requests_per_second: u64,

    #[arg(long, default_value_t = DEFAULT_BMC_BURST)]
    bmc_burst: u64,

    #[arg(long, default_value_t = DEFAULT_REDFISH_TIMEOUT_SECONDS)]
    timeout_seconds: u64,
}

#[derive(Debug, Parser)]
struct RuntimeArgs {
    #[command(flatten)]
    collector: CollectorArgs,

    #[arg(long, env = "FLEETINT_BACKEND_URL")]
    backend_url: Option<String>,

    #[arg(long, env = "FLEETINT_AGENT_TOKEN")]
    agent_token: Option<String>,

    #[arg(long, default_value_t = DEFAULT_INVENTORY_INTERVAL_SECONDS)]
    interval_seconds: u64,
}

#[derive(Debug, Parser)]
struct RenderNicoConfigArgs {
    #[arg(short, long, default_value = "config.toml")]
    config: String,

    #[arg(short, long)]
    output: String,

    #[arg(long, default_value = "http://localhost:4317")]
    otlp_endpoint: String,

    #[arg(long, default_value = "0.0.0.0:9009")]
    metrics_endpoint: String,

    #[arg(long, default_value = "fleetint")]
    metrics_prefix: String,

    #[arg(long, default_value_t = DEFAULT_REDFISH_TIMEOUT_SECONDS)]
    nmxc_timeout_seconds: u64,
}

#[derive(Clone, Copy, Debug, ValueEnum)]
enum CliInventoryFormat {
    Table,
    Json,
}

impl From<CliInventoryFormat> for InventoryFormat {
    fn from(format: CliInventoryFormat) -> Self {
        match format {
            CliInventoryFormat::Table => Self::Table,
            CliInventoryFormat::Json => Self::Json,
        }
    }
}

impl CollectorArgs {
    fn into_inventory_options(self, detail: bool, format: InventoryFormat) -> InventoryOptions {
        InventoryOptions {
            config: self.config,
            insecure_tls: self.insecure_tls,
            detail,
            format,
            target_concurrency: self.target_concurrency,
            bmc_max_in_flight: self.bmc_max_in_flight,
            bmc_requests_per_second: self.bmc_requests_per_second,
            bmc_burst: self.bmc_burst,
            timeout_seconds: self.timeout_seconds,
        }
    }
}

fn write_private_file(path: &str, contents: &str) -> Result<(), Box<dyn Error>> {
    let mut options = OpenOptions::new();
    options.create(true).truncate(true).write(true);
    #[cfg(unix)]
    options.mode(0o640);
    let mut file = options.open(path)?;
    #[cfg(unix)]
    file.set_permissions(std::fs::Permissions::from_mode(0o640))?;
    file.write_all(contents.as_bytes())?;
    Ok(())
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn Error>> {
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new("info")),
        )
        .with_writer(std::io::stderr)
        .with_target(true)
        .compact()
        .init();

    let cli = Cli::parse();
    match cli.command {
        Command::Inventory(args) => {
            let format = args.format.into();
            run_inventory(args.collector.into_inventory_options(args.detail, format)).await?
        }
        Command::FabricInventory(args) => {
            if args.timeout_seconds == 0 {
                return Err("NMX-C timeout must be greater than zero".into());
            }
            let collector_config = config::load_config(&args.config)?;
            let inventories = nmxc_inventory::collect(
                &collector_config.racks,
                std::time::Duration::from_secs(args.timeout_seconds),
            )
            .await;
            if inventories.is_empty() {
                return Err("no [racks.nmxc] endpoints are configured".into());
            }
            let rendered = match args.format {
                CliInventoryFormat::Table => render_nmxc_table(&inventories, args.detail),
                CliInventoryFormat::Json => render_nmxc_json(&inventories)?,
            };
            std::io::stdout().write_all(rendered.as_bytes())?;
            if !rendered.ends_with('\n') {
                std::io::stdout().write_all(b"\n")?;
            }
        }
        Command::Run(args) => {
            let format = InventoryFormat::Json;
            run_runtime(RuntimeOptions {
                inventory: args.collector.into_inventory_options(false, format),
                backend_url: args.backend_url,
                agent_token: args.agent_token,
                interval_seconds: args.interval_seconds,
            })
            .await?
        }
        Command::RenderNicoConfig(args) => {
            if args.nmxc_timeout_seconds == 0 {
                return Err("NMX-C timeout must be greater than zero".into());
            }
            let collector_config = config::load_config(&args.config)?;
            let (domain_uuids, errors) = nmxc_inventory::discover_domain_uuids(
                &collector_config.racks,
                std::time::Duration::from_secs(args.nmxc_timeout_seconds),
            )
            .await;
            if !errors.is_empty() {
                let details = errors
                    .iter()
                    .map(|error| {
                        format!("{} {}: {}", error.rack_id, error.operation, error.message)
                    })
                    .collect::<Vec<_>>()
                    .join("; ");
                return Err(format!(
                    "failed to discover an NVLink domain UUID for NICo config: {details}"
                )
                .into());
            }
            let rendered = config::render_nico_config_with_domains(
                &collector_config,
                &NicoConfigOptions {
                    otlp_endpoint: args.otlp_endpoint,
                    metrics_endpoint: args.metrics_endpoint,
                    metrics_prefix: args.metrics_prefix,
                },
                &domain_uuids,
            )?;
            write_private_file(&args.output, &rendered)?;
        }
    }
    Ok(())
}
