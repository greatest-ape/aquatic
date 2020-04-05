use std::time::{Duration, Instant};
use std::net::SocketAddr;

use rand::Rng;
use rand_distr::Pareto;

use aquatic::bench_utils::*;
use aquatic::handlers::*;
use aquatic::common::*;

use crate::common::*;


const ANNOUNCE_REQUESTS: usize = 100_000;


pub fn bench(
    rng: &mut impl Rng,
    state: &State,
    info_hashes: &Vec<InfoHash>
){
    println!("# benchmark: handle_announce_requests\n");

    println!("generating data..");

    let mut responses = Vec::with_capacity(ANNOUNCE_REQUESTS);

    let mut announce_requests = create_announce_requests(rng, &info_hashes);

    let time = Time(Instant::now());

    for (request, src) in announce_requests.iter() {
        let key = ConnectionKey {
            connection_id: request.connection_id,
            socket_addr: *src,
        };

        state.connections.insert(key, time);
    }

    let announce_requests = announce_requests.drain(..);

    ::std::thread::sleep(Duration::from_secs(1));

    let now = Instant::now();

    println!("running benchmark..");

    handle_announce_requests(
        &state,
        &mut responses,
        announce_requests,
    );

    let duration = Instant::now() - now;

    println!("\nrequests/second: {:.2}", ANNOUNCE_REQUESTS as f64 / (duration.as_millis() as f64 / 1000.0));
    println!("time per request: {:.2}ns", duration.as_nanos() as f64 / ANNOUNCE_REQUESTS as f64);

    let mut total_num_peers = 0.0f64;
    let mut max_num_peers = 0.0f64;
    let mut num_responses: usize = 0;
    
    for (response, _src) in responses.drain(..) {
        if let Response::Announce(response) = response {
            let n = response.peers.len() as f64;

            total_num_peers += n;
            max_num_peers = max_num_peers.max(n);
            num_responses += 1;
        }
    }

    if num_responses != ANNOUNCE_REQUESTS {
        println!("ERROR: only {} responses received", num_responses);
    }

    println!("avg num peers returned: {:.2}", total_num_peers / ANNOUNCE_REQUESTS as f64);
    println!("max num peers returned: {:.2}", max_num_peers);
}



fn create_announce_requests(
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