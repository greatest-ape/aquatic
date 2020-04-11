use std::io::Cursor;
use std::time::Instant;
use std::net::SocketAddr;

use rand::{Rng, SeedableRng, thread_rng, rngs::{SmallRng, StdRng}};

use aquatic::common::*;
use aquatic::handlers::handle_connect_requests;
use bittorrent_udp::converters::*;

use crate::common::*;


const ITERATIONS: usize = 10_000_000;


pub fn bench(
    requests: Vec<([u8; MAX_REQUEST_BYTES], SocketAddr)>
) -> (f64, f64){
    let state = State::new();
    let mut responses = Vec::with_capacity(ITERATIONS);

    let mut buffer = [0u8; MAX_PACKET_SIZE];
    let mut cursor = Cursor::new(buffer.as_mut());
    let mut num_responses: usize = 0;
    let mut dummy = 0u8;

    let mut rng = StdRng::from_rng(thread_rng()).unwrap();

    let now = Instant::now();

    let mut requests: Vec<(ConnectRequest, SocketAddr)> = requests.into_iter()
        .map(|(request_bytes, src)| {
            if let Request::Connect(r) = request_from_bytes(&request_bytes, 255).unwrap() {
                (r, src)
            } else {
                unreachable!()
            }
        })
        .collect();
    
    let requests = requests.drain(..);

    handle_connect_requests(&state, &mut rng, &mut responses, requests);

    for (response, _) in responses.drain(..){
        if let Response::Connect(_) = response {
            num_responses += 1;
        }

        cursor.set_position(0);

        response_to_bytes(&mut cursor, response, IpVersion::IPv4).unwrap();

        dummy ^= cursor.get_ref()[0];
    }

    let duration = Instant::now() - now;

    let requests_per_second = ITERATIONS as f64 / (duration.as_micros() as f64 / 1000000.0);
    let time_per_request = duration.as_nanos() as f64 / ITERATIONS as f64;

    assert_eq!(num_responses, ITERATIONS);

    // println!("\nrequests/second: {:.2}", requests_per_second);
    // println!("time per request: {:.2}ns", time_per_request);

    /*
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
    */

    if dummy == 123u8 {
        println!("dummy info");
    }

    (requests_per_second, time_per_request)
}


pub fn create_requests() -> Vec<(ConnectRequest, SocketAddr)> {
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