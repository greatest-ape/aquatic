use std::net::{SocketAddr, Ipv4Addr, Ipv6Addr};
use std::thread;
use std::sync::{Arc, atomic::Ordering};
use std::time::{Duration, Instant};

use crossbeam_channel::unbounded;
use rand::prelude::*;
use rand_distr::Pareto;

mod common;
mod handler;
mod network;
mod utils;

use common::*;
use utils::*;
use handler::create_random_request;
use network::*;
use handler::run_handler_thread;


#[global_allocator]
static GLOBAL: mimalloc::MiMalloc = mimalloc::MiMalloc;


pub fn main(){
    aquatic_cli_helpers::run_app_with_cli_and_config::<Config>(
        "aquatic: udp bittorrent tracker: load tester",
        run,
    )
}


fn run(config: Config) -> ::anyhow::Result<()> {
    if config.handler.weight_announce + config.handler.weight_scrape == 0 {
        panic!("Error: at least one weight must be larger than zero.");
    }

    println!("Starting client with config: {:#?}", config);
    
    let mut info_hashes = Vec::with_capacity(config.handler.number_of_torrents);

    let mut rng = SmallRng::from_entropy();

    for _ in 0..config.handler.number_of_torrents {
        info_hashes.push(generate_info_hash(&mut rng));
    }

    let pareto = Pareto::new(
        1.0,
        config.handler.torrent_selection_pareto_shape
    ).unwrap();

    let state = LoadTestState {
        info_hashes: Arc::new(info_hashes),
        statistics: Arc::new(Statistics::default()),
        pareto: Arc::new(pareto),
    };

    // Start socket workers

    let mut request_senders = Vec::new();

    for _ in 0..config.num_socket_workers {
        let (sender, receiver) = unbounded();

        request_senders.push(sender);

        let config = config.clone();
        let state = state.clone();

        thread::spawn(move || run_socket_thread(
            &config,
            state,
            receiver,
        ));
    }

    // Bootstrap request cycle by adding a request to each request channel
    for sender in request_senders.iter(){
        let request = create_random_request(
            &config,
            &state,
            &mut thread_rng()
        );

        sender.send(request.into())
            .expect("bootstrap: add initial request to request queue");
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
