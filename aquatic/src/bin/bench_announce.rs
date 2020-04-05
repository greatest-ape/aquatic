use std::time::{Duration, Instant};
use std::net::SocketAddr;

use rand::{Rng, thread_rng, rngs::SmallRng, SeedableRng};
use rand_distr::Pareto;

use aquatic::common::*;
use aquatic::handler::handle_announce_requests;


const ITERATIONS: usize = 5_000_000;
const NUM_INFO_HASHES: usize = ITERATIONS;


fn main(){
    println!("benchmark: handle_announce_requests\n");

    println!("generating data..");

    let state = State::new();
    let mut responses = Vec::new();

    let mut requests = create_requests();

    let time = Time(Instant::now());

    for (request, src) in requests.iter() {
        let key = ConnectionKey {
            connection_id: request.connection_id,
            socket_addr: *src,
        };

        state.connections.insert(key, time);
    }

    let requests = requests.drain(..);

    ::std::thread::sleep(Duration::from_secs(1));

    let now = Instant::now();

    println!("running benchmark..");

    handle_announce_requests(
        &state,
        &mut responses,
        requests,
    );

    let duration = Instant::now() - now;

    println!("\nrequests/second: {:.2}", ITERATIONS as f64 / (duration.as_millis() as f64 / 1000.0));
    println!("time per request: {:.2}ns", duration.as_nanos() as f64 / ITERATIONS as f64);

    let mut total_num_peers = 0.0f64;
    let mut max_num_peers = 0.0f64;
    
    for (response, _src) in responses {
        if let Response::Announce(response) = response {
            let n = response.peers.len() as f64;

            total_num_peers += n;
            max_num_peers = max_num_peers.max(n);
        }
    }

    println!("avg num peers: {:.2}", total_num_peers / ITERATIONS as f64);
    println!("max num peers: {:.2}", max_num_peers);
}


fn create_requests() -> Vec<(AnnounceRequest, SocketAddr)> {
    let mut rng = SmallRng::from_rng(thread_rng()).unwrap();
    let pareto = Pareto::new(1., 6.).unwrap();

    let info_hashes = create_info_hashes(&mut rng);
    let max_index = info_hashes.len() - 1;

    let mut requests = Vec::new();

    for _ in 0..ITERATIONS {
        let info_hash_index = pareto_usize(&mut rng, pareto, max_index);

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


fn create_info_hashes(rng: &mut impl Rng) -> Vec<InfoHash> {
    let mut info_hashes = Vec::new();

    for _ in 0..NUM_INFO_HASHES {
        info_hashes.push(InfoHash(rng.gen()));
    }

    info_hashes
}


fn pareto_usize(
    rng: &mut impl Rng,
    pareto: Pareto<f64>,
    max: usize,
) -> usize {
    let p: f64 = rng.sample(pareto);
    let p = (p.min(101.0f64) - 1.0) / 100.0;

    (p * max as f64) as usize
}