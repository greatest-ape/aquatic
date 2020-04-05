use std::time::{Duration, Instant};
use std::net::SocketAddr;

use rand::{Rng, thread_rng, rngs::SmallRng, SeedableRng};
use rand_distr::Pareto;

use aquatic::common::*;
use aquatic::handler::*;


const PARETO_SHAPE: f64 = 3.0;
const ANNOUNCE_ITERATIONS: usize = 5_000_000;
const SCRAPE_ITERATIONS: usize = 500_000;
const NUM_INFO_HASHES: usize = 500_000;


fn main(){
    println!("benchmark: handle_announce_requests\n");

    println!("generating data..");

    let state = State::new();
    let mut responses = Vec::new();

    let mut rng = SmallRng::from_rng(thread_rng()).unwrap();

    let info_hashes = create_info_hashes(&mut rng);
    let mut announce_requests = create_announce_requests(&mut rng, &info_hashes);

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

    println!("\nrequests/second: {:.2}", ANNOUNCE_ITERATIONS as f64 / (duration.as_millis() as f64 / 1000.0));
    println!("time per request: {:.2}ns", duration.as_nanos() as f64 / ANNOUNCE_ITERATIONS as f64);

    let mut total_num_peers = 0.0f64;
    let mut max_num_peers = 0.0f64;
    
    for (response, _src) in responses.drain(..) {
        if let Response::Announce(response) = response {
            let n = response.peers.len() as f64;

            total_num_peers += n;
            max_num_peers = max_num_peers.max(n);
        }
    }

    println!("avg num peers: {:.2}", total_num_peers / ANNOUNCE_ITERATIONS as f64);
    println!("max num peers: {:.2}", max_num_peers);


    println!("\n\nbenchmark: handle_scrape_requests\n");

    println!("generating data..");

    let mut scrape_requests = create_scrape_requests(&mut rng, &info_hashes);

    let time = Time(Instant::now());

    for (request, src) in scrape_requests.iter() {
        let key = ConnectionKey {
            connection_id: request.connection_id,
            socket_addr: *src,
        };

        state.connections.insert(key, time);
    }

    let scrape_requests = scrape_requests.drain(..);

    ::std::thread::sleep(Duration::from_secs(1));

    let now = Instant::now();

    println!("running benchmark..");

    handle_scrape_requests(
        &state,
        &mut responses,
        scrape_requests,
    );

    let duration = Instant::now() - now;

    println!("\nrequests/second: {:.2}", SCRAPE_ITERATIONS as f64 / (duration.as_millis() as f64 / 1000.0));
    println!("time per request: {:.2}ns", duration.as_nanos() as f64 / SCRAPE_ITERATIONS as f64);

    let mut total_num_peers = 0.0f64;

    for (response, _src) in responses.drain(..){
        if let Response::Scrape(response) = response {
            for stats in response.torrent_stats {
                total_num_peers += f64::from(stats.seeders.0);
                total_num_peers += f64::from(stats.leechers.0);
            }
        }
    }

    println!("avg num peers: {:.2}", total_num_peers / SCRAPE_ITERATIONS as f64);
}


fn create_announce_requests(
    rng: &mut impl Rng,
    info_hashes: &Vec<InfoHash>
) -> Vec<(AnnounceRequest, SocketAddr)> {
    let pareto = Pareto::new(1., PARETO_SHAPE).unwrap();

    let max_index = info_hashes.len() - 1;

    let mut requests = Vec::new();

    for _ in 0..ANNOUNCE_ITERATIONS {
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



fn create_scrape_requests(
    rng: &mut impl Rng,
    info_hashes: &Vec<InfoHash>
) -> Vec<(ScrapeRequest, SocketAddr)> {
    let pareto = Pareto::new(1., PARETO_SHAPE).unwrap();

    let max_index = info_hashes.len() - 1;

    let mut requests = Vec::new();

    for _ in 0..SCRAPE_ITERATIONS {
        let mut request_info_hashes = Vec::new();

        for _ in 0..10 {
            let info_hash_index = pareto_usize(rng, pareto, max_index);
            request_info_hashes.push(info_hashes[info_hash_index])
        }

        let request = ScrapeRequest {
            connection_id: ConnectionId(rng.gen()),
            transaction_id: TransactionId(rng.gen()),
            info_hashes: request_info_hashes,
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