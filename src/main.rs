use std::error::Error;
use std::str::FromStr;

use clap::{Parser, Subcommand};
use fleet_intelligence_oob::{
    DEFAULT_REDFISH_TIMEOUT_SECONDS, InventoryFormat, InventoryOptions, run_inventory,
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

    #[arg(long, default_value = "table")]
    format: CliInventoryFormat,

    #[arg(long, default_value_t = 16)]
    concurrency: usize,

    #[arg(long, default_value_t = DEFAULT_REDFISH_TIMEOUT_SECONDS)]
    timeout_seconds: u64,
}

#[derive(Clone, Copy, Debug)]
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

impl FromStr for CliInventoryFormat {
    type Err = String;

    fn from_str(value: &str) -> Result<Self, Self::Err> {
        match value {
            "table" => Ok(Self::Table),
            "json" => Ok(Self::Json),
            _ => Err(format!(
                "unsupported inventory format {value:?}; use table or json"
            )),
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
    }
    Ok(())
}
