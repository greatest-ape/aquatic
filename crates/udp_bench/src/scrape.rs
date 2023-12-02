use std::net::{Ipv4Addr, SocketAddr, SocketAddrV4};
use std::time::{Duration, Instant};

use aquatic_common::CanonicalSocketAddr;
use crossbeam_channel::{Receiver, Sender};
use indicatif::ProgressIterator;
use rand::Rng;
use rand_distr::Gamma;

use aquatic_udp::common::*;
use aquatic_udp_protocol::*;

use crate::common::*;
use crate::config::BenchConfig;

pub fn bench_scrape_handler(
    bench_config: &BenchConfig,
    request_sender: &Sender<(SocketWorkerIndex, ConnectedRequest, CanonicalSocketAddr)>,
    response_receiver: &Receiver<(ConnectedResponse, CanonicalSocketAddr)>,
    rng: &mut impl Rng,
    info_hashes: &[InfoHash],
) -> (usize, Duration) {
    let requests = create_requests(
        rng,
        info_hashes,
        bench_config.num_scrape_requests,
        bench_config.num_hashes_per_scrape_request,
    );

    let p = 10_000 * bench_config.num_threads; // FIXME: adjust to sharded workers
    let mut num_responses = 0usize;

    let mut dummy: i32 = rng.gen();

    let pb = create_progress_bar("Scrape", bench_config.num_rounds as u64);

    // Start benchmark

    let before = Instant::now();

    for round in (0..bench_config.num_rounds).progress_with(pb) {
        for request_chunk in requests.chunks(p) {
            for (request, src) in request_chunk {
                let request = ConnectedRequest::Scrape(PendingScrapeRequest {
                    slab_key: 0,
                    info_hashes: request
                        .info_hashes
                        .clone()
                        .into_iter()
                        .enumerate()
                        .collect(),
                });

                request_sender
                    .send((SocketWorkerIndex(0), request, *src))
                    .unwrap();
            }

            while let Ok((ConnectedResponse::Scrape(response), _)) = response_receiver.try_recv() {
                num_responses += 1;

                if let Some(stat) = response.torrent_stats.values().last() {
                    dummy ^= stat.leechers.0.get();
                }
            }
        }

        let total = bench_config.num_scrape_requests * (round + 1);

        while num_responses < total {
            if let Ok((ConnectedResponse::Scrape(response), _)) = response_receiver.recv() {
                num_responses += 1;

                if let Some(stat) = response.torrent_stats.values().last() {
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
) -> Vec<(ScrapeRequest, CanonicalSocketAddr)> {
    let gamma = Gamma::new(GAMMA_SHAPE, GAMMA_SCALE).unwrap();

    let max_index = info_hashes.len() - 1;

    let mut requests = Vec::new();

    for _ in 0..number {
        let mut request_info_hashes = Vec::new();

        for _ in 0..hashes_per_request {
            let info_hash_index = gamma_usize(rng, gamma, max_index);
            request_info_hashes.push(info_hashes[info_hash_index])
        }

        let request = ScrapeRequest {
            connection_id: ConnectionId::new(0),
            transaction_id: TransactionId::new(rng.gen()),
            info_hashes: request_info_hashes,
        };

        requests.push((
            request,
            CanonicalSocketAddr::new(SocketAddr::V4(SocketAddrV4::new(Ipv4Addr::LOCALHOST, 1))),
        ));
    }

    requests
}
