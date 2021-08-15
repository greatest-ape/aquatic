use std::net::SocketAddr;
use std::time::{Duration, Instant};

use crossbeam_channel::{Receiver, Sender};
use indicatif::ProgressIterator;
use rand::Rng;
use rand_distr::Pareto;

use aquatic_udp::common::*;
use aquatic_udp::config::Config;

use crate::common::*;
use crate::config::BenchConfig;

pub fn bench_scrape_handler(
    state: &State,
    bench_config: &BenchConfig,
    aquatic_config: &Config,
    request_sender: &Sender<(Request, SocketAddr)>,
    response_receiver: &Receiver<(Response, SocketAddr)>,
    rng: &mut impl Rng,
    info_hashes: &[InfoHash],
) -> (usize, Duration) {
    let requests = create_requests(
        state,
        rng,
        info_hashes,
        bench_config.num_scrape_requests,
        bench_config.num_hashes_per_scrape_request,
    );

    let p = aquatic_config.handlers.max_requests_per_iter * bench_config.num_threads;
    let mut num_responses = 0usize;

    let mut dummy: i32 = rng.gen();

    let pb = create_progress_bar("Scrape", bench_config.num_rounds as u64);

    // Start benchmark

    let before = Instant::now();

    for round in (0..bench_config.num_rounds).progress_with(pb) {
        for request_chunk in requests.chunks(p) {
            for (request, src) in request_chunk {
                request_sender.send((request.clone().into(), *src)).unwrap();
            }

            while let Ok((Response::Scrape(r), _)) = response_receiver.try_recv() {
                num_responses += 1;

                if let Some(stat) = r.torrent_stats.last() {
                    dummy ^= stat.leechers.0;
                }
            }
        }

        let total = bench_config.num_scrape_requests * (round + 1);

        while num_responses < total {
            if let Ok((Response::Scrape(r), _)) = response_receiver.recv() {
                num_responses += 1;

                if let Some(stat) = r.torrent_stats.last() {
                    dummy ^= stat.leechers.0;
                }
            }
        }
    }

    let elapsed = before.elapsed();

    if dummy == 0 {
        println!("dummy dummy");
    }

    (num_responses, elapsed)
}

pub fn create_requests(
    state: &State,
    rng: &mut impl Rng,
    info_hashes: &[InfoHash],
    number: usize,
    hashes_per_request: usize,
) -> Vec<(ScrapeRequest, SocketAddr)> {
    let pareto = Pareto::new(1., PARETO_SHAPE).unwrap();

    let max_index = info_hashes.len() - 1;

    let connections = state.connections.lock();

    let connection_keys: Vec<ConnectionKey> = connections.keys().take(number).cloned().collect();

    let mut requests = Vec::new();

    for connection_key in connection_keys.into_iter() {
        let mut request_info_hashes = Vec::new();

        for _ in 0..hashes_per_request {
            let info_hash_index = pareto_usize(rng, pareto, max_index);
            request_info_hashes.push(info_hashes[info_hash_index])
        }

        let request = ScrapeRequest {
            connection_id: connection_key.connection_id,
            transaction_id: TransactionId(rng.gen()),
            info_hashes: request_info_hashes,
        };

        requests.push((request, connection_key.socket_addr));
    }

    requests
}
