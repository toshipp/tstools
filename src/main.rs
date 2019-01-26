use structopt::StructOpt;

#[macro_use]
mod util;
mod arib;
mod cmd;
mod crc32;
mod pes;
mod psi;
mod ts;

#[derive(StructOpt)]
enum Opt {
    #[structopt(name = "program")]
    Program,
    #[structopt(name = "subtitle")]
    Subtitle,
}

fn main() {
    let opt = Opt::from_args();
    match opt {
        Opt::Program => {
            cmd::dump_program::run();
        }
        Opt::Subtitle => {
            cmd::dump_subtitle::run();
        }
    }
}
