use std::error::Error;

use clap::{Args, Parser, Subcommand, ValueEnum};
use fleet_intelligence_oob::{
    DEFAULT_INVENTORY_INTERVAL_SECONDS, DEFAULT_REDFISH_TIMEOUT_SECONDS,
    DEFAULT_TARGET_CONCURRENCY, InventoryFormat, InventoryOptions, RuntimeOptions, run_inventory,
    run_runtime,
};

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
    #[command(about = "Continuously collect BMC data and export it to Fleet Intelligence")]
    Run(RuntimeArgs),
}

#[derive(Debug, Parser)]
struct InventoryArgs {
    #[command(flatten)]
    collector: CollectorArgs,

    #[arg(long)]
    detail: bool,

    #[arg(long, default_value = "table", value_enum)]
    format: CliInventoryFormat,

    /// Output file required when --format nico-health is selected.
    #[arg(long, value_name = "PATH")]
    output: Option<String>,
}

#[derive(Clone, Debug, Args)]
struct CollectorArgs {
    #[arg(short, long, default_value = "config.toml")]
    config: String,

    #[arg(long)]
    insecure_tls: bool,

    #[arg(long, default_value_t = DEFAULT_TARGET_CONCURRENCY)]
    concurrency: usize,

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

#[derive(Clone, Copy, Debug, ValueEnum)]
enum CliInventoryFormat {
    Table,
    Json,
    NicoHealth,
}

impl From<CliInventoryFormat> for InventoryFormat {
    fn from(format: CliInventoryFormat) -> Self {
        match format {
            CliInventoryFormat::Table => Self::Table,
            CliInventoryFormat::Json => Self::Json,
            CliInventoryFormat::NicoHealth => Self::NicoHealth,
        }
    }
}

impl CollectorArgs {
    fn into_inventory_options(
        self,
        detail: bool,
        format: InventoryFormat,
        output: Option<String>,
    ) -> InventoryOptions {
        InventoryOptions {
            config: self.config,
            insecure_tls: self.insecure_tls,
            detail,
            format,
            concurrency: self.concurrency,
            timeout_seconds: self.timeout_seconds,
            output,
        }
    }
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
            run_inventory(
                args.collector
                    .into_inventory_options(args.detail, format, args.output),
            )
            .await?
        }
        Command::Run(args) => {
            let format = InventoryFormat::Json;
            run_runtime(RuntimeOptions {
                inventory: args.collector.into_inventory_options(false, format, None),
                backend_url: args.backend_url,
                agent_token: args.agent_token,
                interval_seconds: args.interval_seconds,
            })
            .await?
        }
    }
    Ok(())
}
