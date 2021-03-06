use std::net::SocketAddr;
use std::time::{Duration, Instant};

use crossbeam_channel::{Sender, Receiver};
use indicatif::ProgressIterator;
use rand::Rng;
use rand_distr::Pareto;

use aquatic_udp::common::*;
use aquatic_udp::config::Config;

use crate::common::*;
use crate::config::BenchConfig;


pub fn bench_announce_handler(
    state: &State,
    bench_config: &BenchConfig,
    aquatic_config: &Config,
    request_sender: &Sender<(Request, SocketAddr)>,
    response_receiver: &Receiver<(Response, SocketAddr)>,
    rng: &mut impl Rng,
    info_hashes: &[InfoHash],
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
            if let Ok((Response::Announce(r), _)) = response_receiver.recv() {
                num_responses += 1;

                if let Some(last_peer) = r.peers.last(){
                    dummy ^= last_peer.port.0;
                }
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
    info_hashes: &[InfoHash],
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

    for connection_key in connection_keys.into_iter(){
        let info_hash_index = pareto_usize(rng, pareto, max_index);

        let request = AnnounceRequest {
            connection_id: connection_key.connection_id,
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

        requests.push((request, connection_key.socket_addr));
    }

    requests
}