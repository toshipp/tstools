use structopt::StructOpt;

#[macro_use]
mod util;
mod arib;
mod cmd;
mod crc32;
mod pes;
mod psi;
mod stream;
mod ts;

#[derive(StructOpt)]
enum Opt {
    #[structopt(name = "events")]
    Events,
    #[structopt(name = "caption")]
    Caption,
    #[structopt(name = "jitter")]
    Jitter,
}

fn main() {
    let opt = Opt::from_args();
    match opt {
        Opt::Events => {
            cmd::events::run();
        }
        Opt::Caption => {
            cmd::caption::run();
        }
        Opt::Jitter => {
            cmd::jitter::run();
        }
    }
}
