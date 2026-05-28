use std::error::Error;

use clap::{Parser, Subcommand};
use fleet_intelligence_oob::{InventoryOptions, run_inventory};
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
        Command::Inventory(args) => {
            run_inventory(InventoryOptions {
                config: args.config,
                target: args.target,
                port: args.port,
                boot_mac: args.boot_mac,
                insecure_tls: args.insecure_tls,
            })
            .await?
        }
    }
    Ok(())
}
