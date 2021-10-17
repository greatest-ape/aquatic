//! Benchmark announce and scrape handlers
//!
//! Example outputs:
//! ```
//! # Results over 20 rounds with 1 threads
//! Connect:   2 306 637 requests/second,   433.53 ns/request
//! Announce:    688 391 requests/second,  1452.66 ns/request
//! Scrape:    1 505 700 requests/second,   664.14 ns/request
//! ```
//! ```
//! # Results over 20 rounds with 2 threads
//! Connect:   3 472 434 requests/second,   287.98 ns/request
//! Announce:    739 371 requests/second,  1352.50 ns/request
//! Scrape:    1 845 253 requests/second,   541.93 ns/request
//! ```

use crossbeam_channel::unbounded;
use num_format::{Locale, ToFormattedString};
use rand::{rngs::SmallRng, thread_rng, Rng, SeedableRng};
use std::time::Duration;

use aquatic_cli_helpers::run_app_with_cli_and_config;
use aquatic_udp::common::*;
use aquatic_udp::config::Config;
use aquatic_udp::handlers;

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
        run,
        None,
    )
}

pub fn run(bench_config: BenchConfig) -> ::anyhow::Result<()> {
    // Setup common state, spawn request handlers

    let state = State::default();
    let aquatic_config = Config::default();

    let (request_sender, request_receiver) = unbounded();
    let (response_sender, response_receiver) = unbounded();

    for _ in 0..bench_config.num_threads {
        let state = state.clone();
        let config = aquatic_config.clone();
        let request_receiver = request_receiver.clone();
        let response_sender = response_sender.clone();

        ::std::thread::spawn(move || {
            handlers::run_request_worker(state, config, request_receiver, response_sender)
        });
    }

    // Run benchmarks

    let mut rng = SmallRng::from_rng(thread_rng()).unwrap();
    let info_hashes = create_info_hashes(&mut rng);

    let a = announce::bench_announce_handler(
        &bench_config,
        &aquatic_config,
        &request_sender,
        &response_receiver,
        &mut rng,
        &info_hashes,
    );

    let s = scrape::bench_scrape_handler(
        &bench_config,
        &aquatic_config,
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
