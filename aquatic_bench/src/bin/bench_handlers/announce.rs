use std::time::Instant;
use std::net::SocketAddr;

use rand::Rng;
use rand_distr::Pareto;

use aquatic::handlers::*;
use aquatic::common::*;
use aquatic::config::Config;
use aquatic_bench::*;

use crate::common::*;


const ANNOUNCE_REQUESTS: usize = 1_000_000;


pub fn bench(
    state: &State,
    config: &Config,
    requests: Vec<(AnnounceRequest, SocketAddr)>,
) -> (f64, f64) {
    let mut responses = Vec::with_capacity(ANNOUNCE_REQUESTS);
    let mut requests = requests;
    let requests = requests.drain(..);

    let now = Instant::now();

    handle_announce_requests(
        &state,
        config,
        &mut responses,
        requests,
    );

    let duration = Instant::now() - now;

    let requests_per_second = ANNOUNCE_REQUESTS as f64 / (duration.as_millis() as f64 / 1000.0);
    let time_per_request = duration.as_nanos() as f64 / ANNOUNCE_REQUESTS as f64;

    // println!("\nrequests/second: {:.2}", requests_per_second);
    // println!("time per request: {:.2}ns", time_per_request);

    // let mut total_num_peers = 0.0f64;
    let mut num_responses: usize = 0;
    
    for (response, _src) in responses.drain(..) {
        if let Response::Announce(_response) = response {
            // let n = response.peers.len() as f64;

            // total_num_peers += n;
            num_responses += 1;
        }
    }

    if num_responses != ANNOUNCE_REQUESTS {
        println!("ERROR: only {} responses received", num_responses);
    }

    // println!("avg num peers returned: {:.2}", total_num_peers / ANNOUNCE_REQUESTS as f64);

    (requests_per_second, time_per_request)
}



pub fn create_requests(
    rng: &mut impl Rng,
    info_hashes: &Vec<InfoHash>
) -> Vec<(AnnounceRequest, SocketAddr)> {
    let pareto = Pareto::new(1., PARETO_SHAPE).unwrap();

    let max_index = info_hashes.len() - 1;

    let mut requests = Vec::new();

    for _ in 0..ANNOUNCE_REQUESTS {
        let info_hash_index = pareto_usize(rng, pareto, max_index);

        let request = AnnounceRequest {
            connection_id: ConnectionId(rng.gen()),
            transaction_id: TransactionId(rng.gen()),
            info_hash: info_hashes[info_hash_index],
            peer_id: PeerId(rng.gen()),
            bytes_downloaded: NumberOfBytes(rng.gen()),
            bytes_uploaded: NumberOfBytes(rng.gen()),
            bytes_left: NumberOfBytes(rng.gen()),
            event: AnnounceEvent::Started,
            ip_address: None, 
            key: PeerKey(rng.gen()),
            peers_wanted: NumberOfPeers(rng.gen()),
            port: Port(rng.gen())
        };

        let src = SocketAddr::from(([rng.gen(), rng.gen(), rng.gen(), rng.gen()], rng.gen()));

        requests.push((request, src));
    }

    requests
}