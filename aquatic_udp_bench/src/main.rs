//! Benchmark announce and scrape handlers
//!
//! Example outputs:
//! ```
//! # Results over 10 rounds with 2 threads
//! Announce:    429 540 requests/second,  2328.07 ns/request
//! Scrape:    1 873 545 requests/second,   533.75 ns/request
//! ```

use aquatic_common::PanicSentinelWatcher;
use aquatic_udp::workers::request::run_request_worker;
use crossbeam_channel::unbounded;
use num_format::{Locale, ToFormattedString};
use rand::{rngs::SmallRng, thread_rng, Rng, SeedableRng};
use std::time::Duration;

use aquatic_common::cli::run_app_with_cli_and_config;
use aquatic_udp::common::*;
use aquatic_udp::config::Config;
use aquatic_udp_protocol::*;

use config::BenchConfig;

mod announce;
mod common;
mod config;
mod scrape;

#[global_allocator]
static GLOBAL: mimalloc::MiMalloc = mimalloc::MiMalloc;

fn main() {
    run_app_with_cli_and_config::<BenchConfig>(
        "aquatic_udp_bench: Run aquatic_udp benchmarks",
        env!("CARGO_PKG_VERSION"),
        run,
        None,
    )
}

pub fn run(bench_config: BenchConfig) -> ::anyhow::Result<()> {
    // Setup common state, spawn request handlers

    let mut aquatic_config = Config::default();
    let (_, sentinel) = PanicSentinelWatcher::create_with_sentinel();

    aquatic_config.cleaning.torrent_cleaning_interval = 60 * 60 * 24;

    let (request_sender, request_receiver) = unbounded();
    let (response_sender, response_receiver) = unbounded();

    let response_sender = ConnectedResponseSender::new(vec![response_sender]);

    {
        let config = aquatic_config.clone();
        let state = State::new(config.request_workers);

        ::std::thread::spawn(move || {
            run_request_worker(
                sentinel,
                config,
                state,
                request_receiver,
                response_sender,
                RequestWorkerIndex(0),
            )
        });
    }

    // Run benchmarks

    let mut rng = SmallRng::from_rng(thread_rng()).unwrap();
    let info_hashes = create_info_hashes(&mut rng);

    let a = announce::bench_announce_handler(
        &bench_config,
        &request_sender,
        &response_receiver,
        &mut rng,
        &info_hashes,
    );

    let s = scrape::bench_scrape_handler(
        &bench_config,
        &request_sender,
        &response_receiver,
        &mut rng,
        &info_hashes,
    );

    println!(
        "\n# Results over {} rounds with {} threads",
        bench_config.num_rounds, bench_config.num_threads,
    );

    print_results("Announce:", a.0, a.1);
    print_results("Scrape:  ", s.0, s.1);

    Ok(())
}

pub fn print_results(request_type: &str, num_responses: usize, duration: Duration) {
    let per_second = ((num_responses as f64 / (duration.as_micros() as f64 / 1000000.0)) as usize)
        .to_formatted_string(&Locale::se);

    let time_per_request = duration.as_nanos() as f64 / (num_responses as f64);

    println!(
        "{} {:>10} requests/second, {:>8.2} ns/request",
        request_type, per_second, time_per_request,
    );
}

fn create_info_hashes(rng: &mut impl Rng) -> Vec<InfoHash> {
    let mut info_hashes = Vec::new();

    for _ in 0..common::NUM_INFO_HASHES {
        info_hashes.push(InfoHash(rng.gen()));
    }

    info_hashes
}
