use std::time::{Duration, Instant};

use crossbeam_channel::{Receiver, Sender};
use indicatif::ProgressIterator;
use rand::{rngs::SmallRng, thread_rng, Rng, SeedableRng};
use std::net::SocketAddr;

use aquatic_udp::common::*;
use aquatic_udp::config::Config;

use crate::common::*;
use crate::config::BenchConfig;

pub fn bench_connect_handler(
    bench_config: &BenchConfig,
    aquatic_config: &Config,
    request_sender: &Sender<(Request, SocketAddr)>,
    response_receiver: &Receiver<(Response, SocketAddr)>,
) -> (usize, Duration) {
    let requests = create_requests(bench_config.num_connect_requests);

    let p = aquatic_config.handlers.max_requests_per_iter * bench_config.num_threads;
    let mut num_responses = 0usize;

    let mut dummy: i64 = thread_rng().gen();

    let pb = create_progress_bar("Connect", bench_config.num_rounds as u64);

    // Start connect benchmark

    let before = Instant::now();

    for round in (0..bench_config.num_rounds).progress_with(pb) {
        for request_chunk in requests.chunks(p) {
            for (request, src) in request_chunk {
                request_sender.send((request.clone().into(), *src)).unwrap();
            }

            while let Ok((Response::Connect(r), _)) = response_receiver.try_recv() {
                num_responses += 1;
                dummy ^= r.connection_id.0;
            }
        }

        let total = bench_config.num_connect_requests * (round + 1);

        while num_responses < total {
            if let Ok((Response::Connect(r), _)) = response_receiver.recv() {
                num_responses += 1;
                dummy ^= r.connection_id.0;
            }
        }
    }

    let elapsed = before.elapsed();

    if dummy == 0 {
        println!("dummy dummy");
    }

    (num_responses, elapsed)
}

pub fn create_requests(number: usize) -> Vec<(ConnectRequest, SocketAddr)> {
    let mut rng = SmallRng::from_rng(thread_rng()).unwrap();

    let mut requests = Vec::new();

    for _ in 0..number {
        let request = ConnectRequest {
            transaction_id: TransactionId(rng.gen()),
        };

        let src = SocketAddr::from(([rng.gen(), rng.gen(), rng.gen(), rng.gen()], rng.gen()));

        requests.push((request, src));
    }

    requests
}
