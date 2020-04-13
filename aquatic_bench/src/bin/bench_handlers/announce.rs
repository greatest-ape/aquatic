use std::net::SocketAddr;
use std::time::{Duration, Instant};

use crossbeam_channel::{Sender, Receiver};
use indicatif::ProgressIterator;
use rand::Rng;
use rand_distr::Pareto;

use aquatic::common::*;
use aquatic::config::Config;

use aquatic_bench::pareto_usize;

use crate::common::*;
use crate::config::BenchConfig;


pub fn bench_announce_handler(
    state: &State,
    bench_config: &BenchConfig,
    aquatic_config: &Config,
    request_sender: &Sender<(Request, SocketAddr)>,
    response_receiver: &Receiver<(Response, SocketAddr)>,
    rng: &mut impl Rng,
    info_hashes: &Vec<InfoHash>,
) -> (usize, Duration) {
    let requests = create_requests(
        state,
        rng,
        info_hashes,
        bench_config.num_announce_requests
    );

    let p = aquatic_config.handlers.max_requests_per_iter * bench_config.num_threads;
    let mut num_responses = 0usize;

    let mut dummy: u16 = rng.gen();

    let pb = create_progress_bar("Announce", bench_config.num_rounds as u64);

    // Start benchmark

    let before = Instant::now();

    for round in (0..bench_config.num_rounds).progress_with(pb){
        for request_chunk in requests.chunks(p){
            for (request, src) in request_chunk {
                request_sender.send((request.clone().into(), *src)).unwrap();
            }

            while let Ok((Response::Announce(r), _)) = response_receiver.try_recv() {
                num_responses += 1;

                if let Some(last_peer) = r.peers.last(){
                    dummy ^= last_peer.port.0;
                }
            }
        }

        let total = bench_config.num_announce_requests * (round + 1);

        while num_responses < total {
            match response_receiver.recv(){
                Ok((Response::Announce(r), _)) => {
                    num_responses += 1;

                    if let Some(last_peer) = r.peers.last(){
                        dummy ^= last_peer.port.0;
                    }
                },
                _ => {}
            }
        }
    }

    let elapsed = before.elapsed();

    if dummy == 0 {
        println!("dummy dummy");
    }

    (num_responses, elapsed)
}


pub fn create_requests(
    state: &State,
    rng: &mut impl Rng,
    info_hashes: &Vec<InfoHash>,
    number: usize,
) -> Vec<(AnnounceRequest, SocketAddr)> {
    let pareto = Pareto::new(1., PARETO_SHAPE).unwrap();

    let max_index = info_hashes.len() - 1;

    let mut requests = Vec::new();

    let connections = state.connections.lock();

    let connection_keys: Vec<ConnectionKey> = connections.keys()
        .take(number)
        .cloned()
        .collect();

    for i in 0..number {
        let info_hash_index = pareto_usize(rng, pareto, max_index);

        // Will panic if less connection requests than announce requests
        let connection_id = connection_keys[i].connection_id; 
        let src = connection_keys[i].socket_addr;

        let request = AnnounceRequest {
            connection_id,
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

        requests.push((request, src));
    }

    requests
}