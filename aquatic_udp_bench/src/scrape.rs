use std::net::{Ipv4Addr, SocketAddr, SocketAddrV4};
use std::time::{Duration, Instant};

use crossbeam_channel::{Receiver, Sender};
use indicatif::ProgressIterator;
use rand::Rng;
use rand_distr::Pareto;

use aquatic_udp::common::*;
use aquatic_udp::config::Config;
use aquatic_udp_protocol::*;

use crate::common::*;
use crate::config::BenchConfig;

pub fn bench_scrape_handler(
    bench_config: &BenchConfig,
    aquatic_config: &Config,
    request_sender: &Sender<(ConnectedRequest, SocketAddr)>,
    response_receiver: &Receiver<(ConnectedResponse, SocketAddr)>,
    rng: &mut impl Rng,
    info_hashes: &[InfoHash],
) -> (usize, Duration) {
    let requests = create_requests(
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
                let request = ConnectedRequest::Scrape {
                    request: request.clone(),
                    original_indices: Vec::new(),
                };

                request_sender.send((request, *src)).unwrap();
            }

            while let Ok((ConnectedResponse::Scrape { response, .. }, _)) =
                response_receiver.try_recv()
            {
                num_responses += 1;

                if let Some(stat) = response.torrent_stats.last() {
                    dummy ^= stat.leechers.0.get();
                }
            }
        }

        let total = bench_config.num_scrape_requests * (round + 1);

        while num_responses < total {
            if let Ok((ConnectedResponse::Scrape { response, .. }, _)) = response_receiver.recv() {
                num_responses += 1;

                if let Some(stat) = response.torrent_stats.last() {
                    dummy ^= stat.leechers.0.get();
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
    rng: &mut impl Rng,
    info_hashes: &[InfoHash],
    number: usize,
    hashes_per_request: usize,
) -> Vec<(ScrapeRequest, SocketAddr)> {
    let pareto = Pareto::new(1., PARETO_SHAPE).unwrap();

    let max_index = info_hashes.len() - 1;

    let mut requests = Vec::new();

    for _ in 0..number {
        let mut request_info_hashes = Vec::new();

        for _ in 0..hashes_per_request {
            let info_hash_index = pareto_usize(rng, pareto, max_index);
            request_info_hashes.push(info_hashes[info_hash_index])
        }

        let request = ScrapeRequest {
            fixed: ScrapeRequestFixed {
                connection_id: ConnectionId(0.into()),
                action: ScrapeAction::new(),
                transaction_id: TransactionId(rng.gen::<i32>().into()),
            },
            info_hashes: request_info_hashes,
        };

        requests.push((
            request,
            SocketAddr::V4(SocketAddrV4::new(Ipv4Addr::LOCALHOST, 1)),
        ));
    }

    requests
}
