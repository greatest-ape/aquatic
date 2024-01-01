pub mod common;
pub mod html;
pub mod protocols;
pub mod run;
pub mod set;

use clap::{Parser, Subcommand};
use common::CpuMode;

#[derive(Parser)]
#[command(author, version, about)]
struct Args {
    /// How to choose which virtual CPUs to allow trackers and load test
    /// executables on
    #[arg(long, default_value_t = CpuMode::Split)]
    cpu_mode: CpuMode,
    /// Minimum number of tracker cpu cores to run benchmarks for
    #[arg(long)]
    min_cores: Option<usize>,
    /// Maximum number of tracker cpu cores to run benchmarks for
    #[arg(long)]
    max_cores: Option<usize>,
    #[command(subcommand)]
    command: Command,
}

#[derive(Subcommand)]
enum Command {
    /// Benchmark UDP BitTorrent trackers aquatic_udp, opentracker and chihaya
    #[cfg(feature = "udp")]
    Udp(protocols::udp::UdpCommand),
}

fn main() {
    let args = Args::parse();

    match args.command {
        #[cfg(feature = "udp")]
        Command::Udp(command) => command
            .run(args.cpu_mode, args.min_cores, args.max_cores)
            .unwrap(),
    }
}
