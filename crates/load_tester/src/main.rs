pub mod common;
pub mod protocols;
pub mod run;
pub mod set;

use clap::{Parser, Subcommand};

#[derive(Parser)]
#[command(author, version, about)]
struct Args {
    #[command(subcommand)]
    command: Command,
}

#[derive(Subcommand)]
enum Command {
    #[cfg(feature = "udp")]
    Udp(protocols::udp::UdpCommand),
}

fn main() {
    let args = Args::parse();

    match args.command {
        #[cfg(feature = "udp")]
        Command::Udp(command) => command.run().unwrap(),
    }
}
