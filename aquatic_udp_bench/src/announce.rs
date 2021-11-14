use std::net::{Ipv4Addr, SocketAddr, SocketAddrV4};
use std::time::{Duration, Instant};

use crossbeam_channel::{Receiver, Sender};
use indicatif::ProgressIterator;
use rand::Rng;
use rand_distr::Pareto;

use aquatic_udp::common::*;
use aquatic_udp::config::Config;
use aquatic_udp_protocol::*;

use crate::common::*;
use crate::config::BenchConfig;

pub fn bench_announce_handler(
    bench_config: &BenchConfig,
    aquatic_config: &Config,
    request_sender: &Sender<(ConnectedRequest, SocketAddr)>,
    response_receiver: &Receiver<(ConnectedResponse, SocketAddr)>,
    rng: &mut impl Rng,
    info_hashes: &[InfoHash],
) -> (usize, Duration) {
    let requests = create_requests(rng, info_hashes, bench_config.num_announce_requests);

    let p = aquatic_config.handlers.max_requests_per_iter * bench_config.num_threads;
    let mut num_responses = 0usize;

    let mut dummy: u16 = rng.gen();

    let pb = create_progress_bar("Announce", bench_config.num_rounds as u64);

    // Start benchmark

    let before = Instant::now();

    for round in (0..bench_config.num_rounds).progress_with(pb) {
        for request_chunk in requests.chunks(p) {
            for (request, src) in request_chunk {
                request_sender
                    .send((ConnectedRequest::Announce(request.clone()), *src))
                    .unwrap();
            }

            while let Ok((ConnectedResponse::Announce(r), _)) = response_receiver.try_recv() {
                num_responses += 1;

                if let Some(last_peer) = r.peers.last() {
                    dummy ^= last_peer.port.0.get();
                }
            }
        }

        let total = bench_config.num_announce_requests * (round + 1);

        while num_responses < total {
            if let Ok((ConnectedResponse::Announce(r), _)) = response_receiver.recv() {
                num_responses += 1;

                if let Some(last_peer) = r.peers.last() {
                    dummy ^= last_peer.port.0.get();
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
    rng: &mut impl Rng,
    info_hashes: &[InfoHash],
    number: usize,
) -> Vec<(AnnounceRequest, SocketAddr)> {
    let pareto = Pareto::new(1., PARETO_SHAPE).unwrap();

    let max_index = info_hashes.len() - 1;

    let mut requests = Vec::new();

    for _ in 0..number {
        let info_hash_index = pareto_usize(rng, pareto, max_index);

        let request = AnnounceRequest {
            connection_id: ConnectionId(0.into()),
            action: AnnounceAction::new(),
            transaction_id: TransactionId(rng.gen::<i32>().into()),
            info_hash: info_hashes[info_hash_index],
            peer_id: PeerId(rng.gen()),
            bytes_downloaded: NumberOfBytes(rng.gen::<i64>().into()),
            bytes_uploaded: NumberOfBytes(rng.gen::<i64>().into()),
            bytes_left: NumberOfBytes(rng.gen::<i64>().into()),
            event: AnnounceEvent::Started.into(),
            ip_address: [0; 4],
            key: PeerKey(rng.gen::<u32>().into()),
            peers_wanted: NumberOfPeers(rng.gen::<i32>().into()),
            port: Port(rng.gen::<u16>().into()),
        };

        requests.push((
            request,
            SocketAddr::V4(SocketAddrV4::new(Ipv4Addr::LOCALHOST, 1)),
        ));
    }

    requests
}
