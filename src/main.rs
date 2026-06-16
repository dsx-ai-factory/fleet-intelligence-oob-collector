use std::error::Error;
use std::str::FromStr;

use bmc_attestation::config::defaults as attestation_defaults;
use clap::{Args, Parser, Subcommand, ValueEnum};
use fleet_intelligence_oob::{
    AttestationFormat, AttestationOptions,
    DEFAULT_INVENTORY_INTERVAL_SECONDS, DEFAULT_REDFISH_TIMEOUT_SECONDS,
    DEFAULT_TARGET_CONCURRENCY, InventoryFormat, InventoryOptions, RuntimeOptions, run_attestation,
    run_inventory, run_runtime,
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
    #[command(about = "Collect BMC attestation data")]
    Attestation(AttestationArgs),
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
    #[arg(short, long, default_value = ".config")]
    config: String,

    #[arg(long)]
    target: Option<String>,

    #[arg(long)]
    insecure_tls: bool,

    #[arg(long, default_value_t = DEFAULT_TARGET_CONCURRENCY)]
    concurrency: usize,

    #[arg(long, default_value_t = DEFAULT_REDFISH_TIMEOUT_SECONDS)]
    timeout_seconds: u64,
}

#[derive(Debug, Parser)]
struct AttestationArgs {
    #[arg(short, long, default_value_t = attestation_defaults::DEFAULT_CONFIG_PATH.to_string())]
    config: String,

    #[arg(long)]
    target: Option<String>,

    #[arg(long)]
    detail: bool,

    #[arg(long, default_value = "table")]
    format: CliAttestationFormat,
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
}

#[derive(Clone, Copy, Debug)]
enum CliAttestationFormat {
    Table,
    Json,
    Backend,
}

impl From<CliInventoryFormat> for InventoryFormat {
    fn from(format: CliInventoryFormat) -> Self {
        match format {
            CliInventoryFormat::Table => Self::Table,
            CliInventoryFormat::Json => Self::Json,
        }
    }
}

impl From<CliAttestationFormat> for AttestationFormat {
    fn from(format: CliAttestationFormat) -> Self {
        match format {
            CliAttestationFormat::Table => Self::Table,
            CliAttestationFormat::Json => Self::Json,
            CliAttestationFormat::Backend => Self::Backend,
        }
    }
}

impl CollectorArgs {
    fn into_options(self, detail: bool, format: InventoryFormat) -> InventoryOptions {
        InventoryOptions {
            config: self.config,
            target: self.target,
            insecure_tls: self.insecure_tls,
            detail,
            format,
            concurrency: self.concurrency,
            timeout_seconds: self.timeout_seconds,
        }
    }
}

impl FromStr for CliAttestationFormat {
    type Err = String;

    fn from_str(value: &str) -> Result<Self, Self::Err> {
        match value {
            "table" => Ok(Self::Table),
            "json" => Ok(Self::Json),
            "backend" => Ok(Self::Backend),
            _ => Err(format!(
                "unsupported attestation format {value:?}; use table, json, or backend"
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
            let format = args.format.into();
            run_inventory(args.collector.into_options(args.detail, format)).await?
        }
        Command::Run(args) => {
            run_runtime(RuntimeOptions {
                inventory: args.collector.into_options(false, InventoryFormat::Json),
                backend_url: args.backend_url,
                agent_token: args.agent_token,
                interval_seconds: args.interval_seconds,
            })
            .await?
        }
        Command::Attestation(args) => {
            run_attestation(AttestationOptions {
                config: args.config,
                target: args.target,
                detail: args.detail,
                format: args.format.into(),
            })
            .await?
        }
    }
    Ok(())
}
