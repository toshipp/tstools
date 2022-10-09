use std::path::PathBuf;

use anyhow::Result;
use clap::{Parser, Subcommand};
use env_logger;

#[macro_use]
mod util;
mod arib;
mod cmd;
mod crc32;
mod h262;
mod pes;
mod psi;
mod stream;
mod ts;

#[derive(Parser)]
struct Cli {
    #[command(subcommand)]
    command: Command,
}

#[derive(Subcommand)]
enum Command {
    Events {
        input: Option<PathBuf>,
    },
    Caption {
        input: Option<PathBuf>,
        #[arg(long = "drcs-map")]
        drcs_map: Option<PathBuf>,
        #[arg(long = "handle-drcs", value_enum, default_value = "error-exit")]
        handle_drcs: cmd::caption::HandleDRCS,
    },
    Jitter {
        input: Option<PathBuf>,
    },
    Clean {
        input: Option<PathBuf>,
        output: Option<PathBuf>,
        #[arg(long = "service-index")]
        service_index: Option<usize>,
    },
}

#[tokio::main]
async fn main() -> Result<()> {
    env_logger::init();

    let cli = Cli::parse();
    match cli.command {
        Command::Events { input } => cmd::events::run(input).await,
        Command::Caption {
            input,
            drcs_map,
            handle_drcs,
        } => cmd::caption::run(input, drcs_map, handle_drcs).await,
        Command::Jitter { input } => cmd::jitter::run(input).await,
        Command::Clean {
            input,
            output,
            service_index,
        } => cmd::clean::run(input, output, service_index).await,
    }
}
