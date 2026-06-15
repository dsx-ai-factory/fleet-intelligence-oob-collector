use std::error::Error;

use clap::{Parser, Subcommand, ValueEnum};
use fleet_intelligence_oob::{
    DEFAULT_INVENTORY_INTERVAL_SECONDS, DEFAULT_REDFISH_TIMEOUT_SECONDS, InventoryFormat,
    InventoryOptions, RuntimeOptions, run_inventory, run_runtime,
};

#[derive(Debug, Parser)]
#[command(name = "fleet-intelligence-oob")]
#[command(about = "Fleet Intelligence OOB collector prototype backed by nv-redfish")]
struct Cli {
    #[command(subcommand)]
    command: Command,
}

#[derive(Debug, Subcommand)]
enum Command {
    Inventory(InventoryArgs),
    Run(RuntimeArgs),
}

#[derive(Debug, Parser)]
struct InventoryArgs {
    #[arg(short, long, default_value = ".config")]
    config: String,

    #[arg(long)]
    target: Option<String>,

    #[arg(long, default_value_t = 443)]
    port: u16,

    #[arg(long)]
    insecure_tls: bool,

    #[arg(long)]
    detail: bool,

    #[arg(long, default_value = "table", value_enum)]
    format: CliInventoryFormat,

    #[arg(long, default_value_t = 16)]
    concurrency: usize,

    #[arg(long, default_value_t = DEFAULT_REDFISH_TIMEOUT_SECONDS)]
    timeout_seconds: u64,
}

#[derive(Debug, Parser)]
struct RuntimeArgs {
    #[arg(short, long, default_value = ".config")]
    config: String,

    #[arg(long)]
    target: Option<String>,

    #[arg(long, default_value_t = 443)]
    port: u16,

    #[arg(long)]
    insecure_tls: bool,

    #[arg(long, default_value_t = 16)]
    concurrency: usize,

    #[arg(long, default_value_t = DEFAULT_REDFISH_TIMEOUT_SECONDS)]
    timeout_seconds: u64,

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
}

impl From<CliInventoryFormat> for InventoryFormat {
    fn from(format: CliInventoryFormat) -> Self {
        match format {
            CliInventoryFormat::Table => Self::Table,
            CliInventoryFormat::Json => Self::Json,
        }
    }
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn Error>> {
    tracing_subscriber::fmt()
        .with_writer(std::io::stderr)
        .with_target(false)
        .compact()
        .init();

    let cli = Cli::parse();
    match cli.command {
        Command::Inventory(args) => {
            run_inventory(InventoryOptions {
                config: args.config,
                target: args.target,
                port: args.port,
                insecure_tls: args.insecure_tls,
                detail: args.detail,
                format: args.format.into(),
                concurrency: args.concurrency,
                timeout_seconds: args.timeout_seconds,
            })
            .await?
        }
        Command::Run(args) => {
            run_runtime(RuntimeOptions {
                inventory: InventoryOptions {
                    config: args.config,
                    target: args.target,
                    port: args.port,
                    insecure_tls: args.insecure_tls,
                    detail: false,
                    format: InventoryFormat::Json,
                    concurrency: args.concurrency,
                    timeout_seconds: args.timeout_seconds,
                },
                backend_url: args.backend_url,
                agent_token: args.agent_token,
                interval_seconds: args.interval_seconds,
            })
            .await?
        }
    }
    Ok(())
}
