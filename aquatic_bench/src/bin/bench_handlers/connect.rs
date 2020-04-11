use std::io::Cursor;
use std::time::{Duration, Instant};
use std::net::SocketAddr;
use std::sync::Arc;

use rand::{Rng, SeedableRng, thread_rng, rngs::{SmallRng, StdRng}};

use aquatic::common::*;
use aquatic::handlers::handle_connect_requests;
use bittorrent_udp::converters::*;

use crate::common::*;


const ITERATIONS: usize = 10_000_000;


pub fn bench(
    state: State,
    requests: Arc<Vec<([u8; MAX_REQUEST_BYTES], SocketAddr)>>
) -> (usize, Duration){
    let mut responses = Vec::with_capacity(ITERATIONS);

    let mut buffer = [0u8; MAX_PACKET_SIZE];
    let mut cursor = Cursor::new(buffer.as_mut());
    let mut num_responses: usize = 0;
    let mut dummy = 0u8;

    let mut rng = StdRng::from_rng(thread_rng()).unwrap();

    let now = Instant::now();

    let mut requests: Vec<(ConnectRequest, SocketAddr)> = requests.iter()
        .map(|(request_bytes, src)| {
            if let Request::Connect(r) = request_from_bytes(request_bytes, 255).unwrap() {
                (r, *src)
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

    assert_eq!(num_responses, ITERATIONS);

    if dummy == 123u8 {
        println!("dummy info");
    }

    (ITERATIONS, duration)
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