use std::net::{Ipv4Addr, Ipv6Addr, SocketAddr};
use std::sync::atomic::AtomicUsize;
use std::sync::{atomic::Ordering, Arc};
use std::thread::{self, Builder};
use std::time::{Duration, Instant};

#[cfg(feature = "cpu-pinning")]
use aquatic_common::cpu_pinning::{pin_current_if_configured_to, WorkerIndex};
use rand_distr::Gamma;

pub mod common;
pub mod config;
pub mod utils;
pub mod worker;

use common::*;
use config::Config;
use utils::*;
use worker::*;

impl aquatic_common::cli::Config for Config {
    fn get_log_level(&self) -> Option<aquatic_common::cli::LogLevel> {
        Some(self.log_level)
    }
}

pub fn run(config: Config) -> ::anyhow::Result<()> {
    if config.requests.weight_announce
        + config.requests.weight_connect
        + config.requests.weight_scrape
        == 0
    {
        panic!("Error: at least one weight must be larger than zero.");
    }

    if config.summarize_last > config.duration {
        panic!("Error: report_last_seconds can't be larger than duration");
    }

    println!("Starting client with config: {:#?}", config);

    let mut info_hashes = Vec::with_capacity(config.requests.number_of_torrents);

    for _ in 0..config.requests.number_of_torrents {
        info_hashes.push(generate_info_hash());
    }

    let state = LoadTestState {
        info_hashes: Arc::from(info_hashes.into_boxed_slice()),
        statistics: Arc::new(Statistics::default()),
    };

    let gamma = Gamma::new(
        config.requests.torrent_gamma_shape,
        config.requests.torrent_gamma_scale,
    )
    .unwrap();

    // Start workers

    for i in 0..config.workers {
        let port = config.network.first_port + (i as u16);

        let ip = if config.server_address.is_ipv6() {
            Ipv6Addr::LOCALHOST.into()
        } else {
            if config.network.multiple_client_ipv4s {
                Ipv4Addr::new(127, 0, 0, 1 + i).into()
            } else {
                Ipv4Addr::LOCALHOST.into()
            }
        };

        let addr = SocketAddr::new(ip, port);
        let config = config.clone();
        let state = state.clone();

        Builder::new().name("load-test".into()).spawn(move || {
            #[cfg(feature = "cpu-pinning")]
            pin_current_if_configured_to(
                &config.cpu_pinning,
                config.workers as usize,
                0,
                WorkerIndex::SocketWorker(i as usize),
            );

            Worker::run(state, gamma, config, addr)
        })?;
    }

    #[cfg(feature = "cpu-pinning")]
    pin_current_if_configured_to(
        &config.cpu_pinning,
        config.workers as usize,
        0,
        WorkerIndex::Util,
    );

    monitor_statistics(state, &config);

    Ok(())
}

fn monitor_statistics(state: LoadTestState, config: &Config) {
    let mut report_avg_connect: Vec<f64> = Vec::new();
    let mut report_avg_announce: Vec<f64> = Vec::new();
    let mut report_avg_scrape: Vec<f64> = Vec::new();
    let mut report_avg_error: Vec<f64> = Vec::new();

    const INTERVAL: u64 = 5;

    let start_time = Instant::now();
    let duration = Duration::from_secs(config.duration as u64);

    let mut last = start_time;

    let time_elapsed = loop {
        thread::sleep(Duration::from_secs(INTERVAL));

        let requests = fetch_and_reset(&state.statistics.requests);
        let response_peers = fetch_and_reset(&state.statistics.response_peers);
        let responses_connect = fetch_and_reset(&state.statistics.responses_connect);
        let responses_announce = fetch_and_reset(&state.statistics.responses_announce);
        let responses_scrape = fetch_and_reset(&state.statistics.responses_scrape);
        let responses_error = fetch_and_reset(&state.statistics.responses_error);

        let now = Instant::now();

        let elapsed = (now - last).as_secs_f64();

        last = now;

        let peers_per_announce_response = response_peers / responses_announce;

        let avg_requests = requests / elapsed;
        let avg_responses_connect = responses_connect / elapsed;
        let avg_responses_announce = responses_announce / elapsed;
        let avg_responses_scrape = responses_scrape / elapsed;
        let avg_responses_error = responses_error / elapsed;

        let avg_responses = avg_responses_connect
            + avg_responses_announce
            + avg_responses_scrape
            + avg_responses_error;

        report_avg_connect.push(avg_responses_connect);
        report_avg_announce.push(avg_responses_announce);
        report_avg_scrape.push(avg_responses_scrape);
        report_avg_error.push(avg_responses_error);

        println!();
        println!("Requests out: {:.2}/second", avg_requests);
        println!("Responses in: {:.2}/second", avg_responses);
        println!("  - Connect responses:  {:.2}", avg_responses_connect);
        println!("  - Announce responses: {:.2}", avg_responses_announce);
        println!("  - Scrape responses:   {:.2}", avg_responses_scrape);
        println!("  - Error responses:    {:.2}", avg_responses_error);
        println!(
            "Peers per announce response: {:.2}",
            peers_per_announce_response
        );

        let time_elapsed = start_time.elapsed();

        if config.duration != 0 && time_elapsed >= duration {
            break time_elapsed;
        }
    };

    if config.summarize_last != 0 {
        let split_at = (config.duration - config.summarize_last) / INTERVAL as usize;

        report_avg_connect = report_avg_connect.split_off(split_at);
        report_avg_announce = report_avg_announce.split_off(split_at);
        report_avg_scrape = report_avg_scrape.split_off(split_at);
        report_avg_error = report_avg_error.split_off(split_at);
    }

    let len = report_avg_connect.len() as f64;

    let avg_connect: f64 = report_avg_connect.into_iter().sum::<f64>() / len;
    let avg_announce: f64 = report_avg_announce.into_iter().sum::<f64>() / len;
    let avg_scrape: f64 = report_avg_scrape.into_iter().sum::<f64>() / len;
    let avg_error: f64 = report_avg_error.into_iter().sum::<f64>() / len;

    let avg_total = avg_connect + avg_announce + avg_scrape + avg_error;

    println!();
    println!("# aquatic load test report");
    println!();
    println!(
        "Test ran for {} seconds {}",
        time_elapsed.as_secs(),
        if config.summarize_last != 0 {
            format!("(only last {} included in summary)", config.summarize_last)
        } else {
            "".to_string()
        }
    );
    println!("Average responses per second: {:.2}", avg_total);
    println!("  - Connect responses:  {:.2}", avg_connect);
    println!("  - Announce responses: {:.2}", avg_announce);
    println!("  - Scrape responses:   {:.2}", avg_scrape);
    println!("  - Error responses:    {:.2}", avg_error);
    println!();
    println!("Config: {:#?}", config);
    println!();
}

fn fetch_and_reset(atomic_usize: &AtomicUsize) -> f64 {
    atomic_usize.fetch_and(0, Ordering::Relaxed) as f64
}
