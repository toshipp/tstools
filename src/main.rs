use std::path::PathBuf;

use env_logger;
use failure::Error;
use structopt::StructOpt;

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

#[derive(StructOpt)]
enum Opt {
    #[structopt(name = "events")]
    Events { input: Option<PathBuf> },
    #[structopt(name = "caption")]
    Caption { input: Option<PathBuf> },
    #[structopt(name = "jitter")]
    Jitter { input: Option<PathBuf> },
    #[structopt(name = "clean")]
    Clean {
        input: Option<PathBuf>,
        output: Option<PathBuf>,
    },
}

fn main() -> Result<(), Error> {
    env_logger::init();

    let opt = Opt::from_args();
    match opt {
        Opt::Events { input } => cmd::events::run(input),
        Opt::Caption { input } => cmd::caption::run(input),
        Opt::Jitter { input } => cmd::jitter::run(input),
        Opt::Clean { input, output } => cmd::clean::run(input, output),
    }
}
