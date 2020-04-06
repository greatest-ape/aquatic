use std::time::{Instant, Duration};
use std::net::SocketAddr;

use rand::{Rng, thread_rng, rngs::SmallRng, SeedableRng};

use aquatic::common::*;
use aquatic::handlers::handle_connect_requests;


const ITERATIONS: usize = 10_000_000;


pub fn bench() -> (f64, f64){
    println!("## benchmark: handle_connect_requests\n");

    let state = State::new();
    let mut responses = Vec::new();

    let mut requests = create_requests();
    let requests = requests.drain(..);

    println!("running benchmark..");

    ::std::thread::sleep(Duration::from_millis(100));

    let now = Instant::now();

    handle_connect_requests(&state, &mut responses, requests);

    let duration = Instant::now() - now;

    let requests_per_second = ITERATIONS as f64 / (duration.as_millis() as f64 / 1000.0);
    let time_per_request = duration.as_nanos() as f64 / ITERATIONS as f64;

    println!("\nrequests/second: {:.2}", requests_per_second);
    println!("time per request: {:.2}ns", time_per_request);

    let mut dummy = 0usize;
    let mut num_responses: usize = 0;
    
    for (response, _src) in responses {
        if let Response::Connect(response) = response {
            if response.connection_id.0 > 0 {
                dummy += 1;
            }

            num_responses += 1;
        }
    }

    if num_responses != ITERATIONS {
        println!("ERROR: only {} responses received", num_responses);
    }

    if dummy == ITERATIONS {
        println!("dummy test output: {}", dummy);
    }

    (requests_per_second, time_per_request)
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