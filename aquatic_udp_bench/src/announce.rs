use std::net::{Ipv4Addr, SocketAddr, SocketAddrV4};
use std::time::{Duration, Instant};

use aquatic_common::CanonicalSocketAddr;
use crossbeam_channel::{Receiver, Sender};
use indicatif::ProgressIterator;
use rand::Rng;
use rand_distr::Pareto;

use aquatic_udp::common::*;
use aquatic_udp_protocol::*;

use crate::common::*;
use crate::config::BenchConfig;

pub fn bench_announce_handler(
    bench_config: &BenchConfig,
    request_sender: &Sender<(SocketWorkerIndex, ConnectedRequest, CanonicalSocketAddr)>,
    response_receiver: &Receiver<(ConnectedResponse, CanonicalSocketAddr)>,
    rng: &mut impl Rng,
    info_hashes: &[InfoHash],
) -> (usize, Duration) {
    let requests = create_requests(rng, info_hashes, bench_config.num_announce_requests);

    let p = 10_000 * bench_config.num_threads; // FIXME: adjust to sharded workers
    let mut num_responses = 0usize;

    let mut dummy: u16 = rng.gen();

    let pb = create_progress_bar("Announce", bench_config.num_rounds as u64);

    // Start benchmark

    let before = Instant::now();

    for round in (0..bench_config.num_rounds).progress_with(pb) {
        for request_chunk in requests.chunks(p) {
            for (request, src) in request_chunk {
                request_sender
                    .send((
                        SocketWorkerIndex(0),
                        ConnectedRequest::Announce(request.clone(), RequestTag::placeholder()),
                        *src,
                    ))
                    .unwrap();
            }

            while let Ok((ConnectedResponse::AnnounceIpv4(r, _), _)) = response_receiver.try_recv()
            {
                num_responses += 1;

                if let Some(last_peer) = r.peers.last() {
                    dummy ^= last_peer.port.0;
                }
            }
        }

        let total = bench_config.num_announce_requests * (round + 1);

        while num_responses < total {
            if let Ok((ConnectedResponse::AnnounceIpv4(r, _), _)) = response_receiver.recv() {
                num_responses += 1;

                if let Some(last_peer) = r.peers.last() {
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
    rng: &mut impl Rng,
    info_hashes: &[InfoHash],
    number: usize,
) -> Vec<(AnnounceRequest, CanonicalSocketAddr)> {
    let pareto = Pareto::new(1., PARETO_SHAPE).unwrap();

    let max_index = info_hashes.len() - 1;

    let mut requests = Vec::new();

    for _ in 0..number {
        let info_hash_index = pareto_usize(rng, pareto, max_index);

        let request = AnnounceRequest {
            connection_id: ConnectionId(0),
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
            port: Port(rng.gen()),
        };

        requests.push((
            request,
            CanonicalSocketAddr::new(SocketAddr::V4(SocketAddrV4::new(Ipv4Addr::LOCALHOST, 1))),
        ));
    }

    requests
}
