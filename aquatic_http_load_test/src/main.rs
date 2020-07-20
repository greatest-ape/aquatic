use std::thread;
use std::sync::{Arc, atomic::Ordering};
use std::time::{Duration, Instant};

use rand::prelude::*;
use rand_distr::Pareto;

mod common;
mod config;
mod network;
mod utils;

use common::*;
use config::*;
use network::*;


#[global_allocator]
static GLOBAL: mimalloc::MiMalloc = mimalloc::MiMalloc;


/// Multiply bytes during a second with this to get Mbit/s
const MBITS_FACTOR: f64 = 1.0 / ((1024.0 * 1024.0) / 8.0);


pub fn main(){
    aquatic_cli_helpers::run_app_with_cli_and_config::<Config>(
        "aquatic: udp bittorrent tracker: load tester",
        run,
    )
}


fn run(config: Config) -> ::anyhow::Result<()> {
    if config.torrents.weight_announce + config.torrents.weight_scrape == 0 {
        panic!("Error: at least one weight must be larger than zero.");
    }

    println!("Starting client with config: {:#?}", config);
    
    let mut info_hashes = Vec::with_capacity(config.torrents.number_of_torrents);

    let mut rng = SmallRng::from_entropy();

    for _ in 0..config.torrents.number_of_torrents {
        info_hashes.push(InfoHash(rng.gen()));
    }

    let pareto = Pareto::new(
        1.0,
        config.torrents.torrent_selection_pareto_shape
    ).unwrap();

    let state = LoadTestState {
        info_hashes: Arc::new(info_hashes),
        statistics: Arc::new(Statistics::default()),
        pareto: Arc::new(pareto),
    };

    // Start socket workers

    for _ in 0..config.num_workers {

        let config = config.clone();
        let state = state.clone();

        thread::spawn(move || run_socket_thread(
            &config,
            state,
            1
        ));
    }

    monitor_statistics(
        state,
        &config
    );

    Ok(())
}


fn monitor_statistics(
    state: LoadTestState,
    config: &Config,
){
    let start_time = Instant::now();
    let mut report_avg_response_vec: Vec<f64> = Vec::new();

    let interval = 5;
    let interval_f64 = interval as f64;

    loop {
        thread::sleep(Duration::from_secs(interval));

        let statistics = state.statistics.as_ref();

        let responses_announce = statistics.responses_announce
            .fetch_and(0, Ordering::SeqCst) as f64;
        let response_peers = statistics.response_peers
            .fetch_and(0, Ordering::SeqCst) as f64;

        let requests_per_second = statistics.requests
            .fetch_and(0, Ordering::SeqCst) as f64 / interval_f64;
        let responses_scrape_per_second = statistics.responses_scrape
            .fetch_and(0, Ordering::SeqCst) as f64 / interval_f64;
        let responses_failure_per_second = statistics.responses_failure
            .fetch_and(0, Ordering::SeqCst) as f64 / interval_f64;

        let bytes_sent_per_second = statistics.bytes_sent
            .fetch_and(0, Ordering::SeqCst) as f64 / interval_f64;
        let bytes_received_per_second = statistics.bytes_received
            .fetch_and(0, Ordering::SeqCst) as f64 / interval_f64;

        let responses_announce_per_second =  responses_announce / interval_f64;

        let responses_per_second = 
            responses_announce_per_second +
            responses_scrape_per_second +
            responses_failure_per_second;

        report_avg_response_vec.push(responses_per_second);

        println!();
        println!("Requests out: {:.2}/second", requests_per_second);
        println!("Responses in: {:.2}/second", responses_per_second);
        println!("  - Announce responses: {:.2}", responses_announce_per_second);
        println!("  - Scrape responses:   {:.2}", responses_scrape_per_second);
        println!("  - Failure responses:  {:.2}", responses_failure_per_second);
        println!("Peers per announce response: {:.2}", response_peers / responses_announce);
        println!("Bandwidth out: {:.2}Mbit/s", bytes_sent_per_second * MBITS_FACTOR);
        println!("Bandwidth in:  {:.2}Mbit/s", bytes_received_per_second * MBITS_FACTOR);
        
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

            break
        }
    }
}
