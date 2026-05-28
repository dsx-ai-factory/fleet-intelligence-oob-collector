mod config;
mod inventory;

use std::error::Error;

use clap::{Parser, Subcommand};
use inventory::mapper::NicoMapper;
use inventory::sink::TableSink;
use inventory::source::RedfishSource;
use mac_address::MacAddress;

#[derive(Debug, Parser)]
#[command(name = "fleet-intelligence-oob")]
#[command(about = "Path A OOB collector prototype backed by NICo crates")]
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
    boot_mac: Option<MacAddress>,

    #[arg(long)]
    insecure_tls: bool,
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
        Command::Inventory(args) => run_inventory(args).await?,
    }
    Ok(())
}

async fn run_inventory(args: InventoryArgs) -> Result<(), Box<dyn Error>> {
    let mut targets = config::load_targets(&args.config)?;
    if let Some(target_name) = &args.target {
        targets.retain(|target| target.name == *target_name);
        if targets.is_empty() {
            return Err(format!("target {target_name:?} not found").into());
        }
    }

    let source = RedfishSource::new(args.port, args.insecure_tls, args.boot_mac);
    let mapper = NicoMapper::default();
    let sink = TableSink::default();

    let mut inventories = Vec::with_capacity(targets.len());
    for target in targets {
        let source_record = source.collect(target).await;
        inventories.push(mapper.map(source_record));
    }

    sink.write(&inventories);
    Ok(())
}
