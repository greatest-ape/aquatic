use std::net::{Ipv4Addr, Ipv6Addr, SocketAddr};
use std::sync::{atomic::Ordering, Arc};
use std::thread;
use std::time::{Duration, Instant};

#[cfg(feature = "cpu-pinning")]
use aquatic_common::cpu_pinning::{pin_current_if_configured_to, WorkerIndex};
use rand_distr::Pareto;

mod common;
mod config;
mod handler;
mod network;
mod utils;

use common::*;
use config::Config;
use network::*;
use utils::*;

#[global_allocator]
static GLOBAL: mimalloc::MiMalloc = mimalloc::MiMalloc;

pub fn main() {
    aquatic_cli_helpers::run_app_with_cli_and_config::<Config>(
        "aquatic_udp_load_test: BitTorrent load tester",
        run,
        None,
    )
}

impl aquatic_cli_helpers::Config for Config {
    fn get_log_level(&self) -> Option<aquatic_cli_helpers::LogLevel> {
        Some(self.log_level)
    }
}

fn run(config: Config) -> ::anyhow::Result<()> {
    if config.handler.weight_announce + config.handler.weight_connect + config.handler.weight_scrape
        == 0
    {
        panic!("Error: at least one weight must be larger than zero.");
    }

    println!("Starting client with config: {:#?}", config);

    let mut info_hashes = Vec::with_capacity(config.handler.number_of_torrents);

    for _ in 0..config.handler.number_of_torrents {
        info_hashes.push(generate_info_hash());
    }

    let state = LoadTestState {
        info_hashes: Arc::new(info_hashes),
        statistics: Arc::new(Statistics::default()),
    };

    let pareto = Pareto::new(1.0, config.handler.torrent_selection_pareto_shape).unwrap();

    // Start workers

    for i in 0..config.workers {
        let thread_id = ThreadId(i);
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

        thread::spawn(move || {
            #[cfg(feature = "cpu-pinning")]
            pin_current_if_configured_to(
                &config.cpu_pinning,
                config.workers as usize,
                WorkerIndex::SocketWorker(i as usize),
            );

            run_worker_thread(state, pareto, &config, addr, thread_id)
        });
    }

    #[cfg(feature = "cpu-pinning")]
    pin_current_if_configured_to(
        &config.cpu_pinning,
        config.workers as usize,
        WorkerIndex::Other,
    );

    monitor_statistics(state, &config);

    Ok(())
}

fn monitor_statistics(state: LoadTestState, config: &Config) {
    let start_time = Instant::now();
    let mut report_avg_response_vec: Vec<f64> = Vec::new();

    let interval = 5;
    let interval_f64 = interval as f64;

    loop {
        thread::sleep(Duration::from_secs(interval));

        let statistics = state.statistics.as_ref();

        let responses_announce =
            statistics.responses_announce.fetch_and(0, Ordering::SeqCst) as f64;
        let response_peers = statistics.response_peers.fetch_and(0, Ordering::SeqCst) as f64;

        let requests_per_second =
            statistics.requests.fetch_and(0, Ordering::SeqCst) as f64 / interval_f64;
        let responses_connect_per_second =
            statistics.responses_connect.fetch_and(0, Ordering::SeqCst) as f64 / interval_f64;
        let responses_scrape_per_second =
            statistics.responses_scrape.fetch_and(0, Ordering::SeqCst) as f64 / interval_f64;
        let responses_error_per_second =
            statistics.responses_error.fetch_and(0, Ordering::SeqCst) as f64 / interval_f64;

        let responses_announce_per_second = responses_announce / interval_f64;

        let responses_per_second = responses_connect_per_second
            + responses_announce_per_second
            + responses_scrape_per_second
            + responses_error_per_second;

        report_avg_response_vec.push(responses_per_second);

        println!();
        println!("Requests out: {:.2}/second", requests_per_second);
        println!("Responses in: {:.2}/second", responses_per_second);
        println!(
            "  - Connect responses:  {:.2}",
            responses_connect_per_second
        );
        println!(
            "  - Announce responses: {:.2}",
            responses_announce_per_second
        );
        println!("  - Scrape responses:   {:.2}", responses_scrape_per_second);
        println!("  - Error responses:    {:.2}", responses_error_per_second);
        println!(
            "Peers per announce response: {:.2}",
            response_peers / responses_announce
        );

        let time_elapsed = start_time.elapsed();
        let duration = Duration::from_secs(config.duration as u64);

        if config.duration != 0 && time_elapsed >= duration {
            let report_len = report_avg_response_vec.len() as f64;
            let report_sum: f64 = report_avg_response_vec.into_iter().sum();
            let report_avg: f64 = report_sum / report_len;

            println!(
                concat!(
                    "\n# aquatic load test report\n\n",
                    "Test ran for {} seconds.\n",
                    "Average responses per second: {:.2}\n\nConfig: {:#?}\n"
                ),
                time_elapsed.as_secs(),
                report_avg,
                config
            );

            break;
        }
    }
}
