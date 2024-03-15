pub mod common;
pub mod html;
pub mod protocols;
pub mod run;
pub mod set;

use clap::{Parser, Subcommand};
use common::{CpuMode, Priority};
use set::run_sets;

#[derive(Parser)]
#[command(author, version, about)]
struct Args {
    /// How to choose which virtual CPUs to allow trackers and load test
    /// executables on
    #[arg(long, default_value_t = CpuMode::SplitPairs)]
    cpu_mode: CpuMode,
    /// Minimum number of tracker cpu cores to run benchmarks for
    #[arg(long)]
    min_cores: Option<usize>,
    /// Maximum number of tracker cpu cores to run benchmarks for
    #[arg(long)]
    max_cores: Option<usize>,
    /// Minimum benchmark priority
    #[arg(long, default_value_t = Priority::Medium)]
    min_priority: Priority,
    /// How long to run each load test for
    #[arg(long, default_value_t = 30)]
    duration: usize,
    /// Only include data for last N seconds of load test runs.
    ///
    /// Useful if the tracker/load tester combination is slow at reaching
    /// maximum throughput
    ///
    /// 0 = use data for whole run
    #[arg(long, default_value_t = 0)]
    summarize_last: usize,
    #[command(subcommand)]
    command: Command,
}

#[derive(Subcommand)]
enum Command {
    /// Benchmark UDP BitTorrent trackers aquatic_udp, opentracker, chihaya and torrust-tracker
    #[cfg(feature = "udp")]
    Udp(protocols::udp::UdpCommand),
}

fn main() {
    let args = Args::parse();

    match args.command {
        #[cfg(feature = "udp")]
        Command::Udp(command) => {
            let sets = command.sets(args.cpu_mode);
            let load_test_gen = protocols::udp::UdpCommand::load_test_gen;

            run_sets(
                &command,
                args.cpu_mode,
                args.min_cores,
                args.max_cores,
                args.min_priority,
                args.duration,
                args.summarize_last,
                sets,
                load_test_gen,
            );
        }
    }
}
