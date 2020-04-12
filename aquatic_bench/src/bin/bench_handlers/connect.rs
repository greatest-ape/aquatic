use std::time::Instant;

use crossbeam_channel::unbounded;
use indicatif::ProgressIterator;
use rand::{Rng, SeedableRng, thread_rng, rngs::SmallRng};
use std::net::SocketAddr;

use aquatic::handlers;
use aquatic::common::*;
use aquatic::config::Config;

use crate::common::*;
use crate::config::BenchConfig;


pub fn bench_connect_handler(bench_config: BenchConfig){
    // Setup common state, spawn request handlers

    let state = State::new();
    let aquatic_config = Config::default();

    let (request_sender, request_receiver) = unbounded();
    let (response_sender, response_receiver) = unbounded();

    for _ in 0..bench_config.num_threads {
        let state = state.clone();
        let config = aquatic_config.clone();
        let request_receiver = request_receiver.clone();
        let response_sender = response_sender.clone();

        ::std::thread::spawn(move || {
            handlers::run_request_worker(
                state,
                config,
                request_receiver,
                response_sender
            )
        });
    }

    // Setup connect benchmark data

    let requests = create_requests(
        bench_config.num_connect_requests
    );

    let p = aquatic_config.handlers.max_requests_per_iter * bench_config.num_threads;
    let mut num_responses = 0usize;

    let mut dummy: i64 = thread_rng().gen();

    let pb = create_progress_bar("Connect handler", bench_config.num_rounds as u64);

    // Start connect benchmark

    let before = Instant::now();

    for round in (0..bench_config.num_rounds).progress_with(pb){
        for request_chunk in requests.chunks(p){
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
            match response_receiver.recv(){
                Ok((Response::Connect(r), _)) => {
                    num_responses += 1;
                    dummy ^= r.connection_id.0;
                },
                _ => {}
            }
        }
    }

    let elapsed = before.elapsed();

    print_results("Connect handler:", num_responses, elapsed);

    if dummy == 0 {
        println!("dummy dummy");
    }
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
