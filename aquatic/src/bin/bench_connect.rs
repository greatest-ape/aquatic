use std::time::Instant;
use std::net::SocketAddr;

use rand::{Rng, thread_rng, rngs::SmallRng, SeedableRng};

use aquatic::common::*;
use aquatic::handler::handle_connect_requests;


const ITERATIONS: usize = 10_000_000;


fn main(){
    println!("benchmark: handle_connect_requests\n");

    let state = State::new();
    let mut responses = Vec::new();

    let mut requests = create_requests();
    let requests = requests.drain(..);

    println!("running benchmark..");

    let now = Instant::now();

    handle_connect_requests(&state, &mut responses, requests);

    let duration = Instant::now() - now;

    println!("\nrequests/second: {:.2}", ITERATIONS as f64 / (duration.as_millis() as f64 / 1000.0));
    println!("time per request: {:.2}ns", duration.as_nanos() as f64 / ITERATIONS as f64);

    let mut dummy = 0usize;
    
    for (response, _src) in responses {
        if let Response::Connect(response) = response {
            if response.connection_id.0 > 0 {
                dummy += 1;
            }
        }
    }

    if dummy == ITERATIONS {
        println!("dummy test output: {}", dummy);
    }
}


fn create_requests() -> Vec<(ConnectRequest, SocketAddr)> {
    let mut rng = SmallRng::from_rng(thread_rng()).unwrap();

    let mut requests = Vec::new();

    for _ in 0..ITERATIONS {
        let request = ConnectRequest {
            transaction_id: TransactionId(rng.gen()),
        };

        let src = SocketAddr::from(([rng.gen(), rng.gen(), rng.gen(), rng.gen()], rng.gen()));

        requests.push((request, src));
    }

    requests
}