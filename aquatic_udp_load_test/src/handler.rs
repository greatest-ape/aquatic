use std::sync::Arc;
use std::time::Duration;
use std::vec::Drain;

use crossbeam_channel::{Receiver, Sender};
use parking_lot::MutexGuard;
use rand::distributions::WeightedIndex;
use rand::prelude::*;
use rand_distr::Pareto;

use aquatic_udp_protocol::types::*;

use crate::common::*;
use crate::utils::*;


pub fn run_handler_thread(
    config: &Config,
    state: LoadTestState,
    pareto: Pareto<f64>,
    request_senders: Vec<Sender<Request>>,
    response_receiver: Receiver<(ThreadId, Response)>,
){
    let state = &state;

    let mut rng1 = SmallRng::from_rng(thread_rng())
        .expect("create SmallRng from thread_rng()");
    let mut rng2 = SmallRng::from_rng(thread_rng())
        .expect("create SmallRng from thread_rng()");
    
    let timeout = Duration::from_micros(config.handler.channel_timeout);

    let mut responses = Vec::new();

    loop {
        let mut opt_torrent_peers = None;

        // Collect a maximum number of responses. Stop collecting before that
        // number is reached if having waited for too long for a request, but
        // only if ConnectionMap mutex isn't locked.
        for i in 0..config.handler.max_responses_per_iter {
            let response = if i == 0 {
                match response_receiver.recv(){
                    Ok(r) => r,
                    Err(_) => break, // Really shouldn't happen
                }
            } else {
                match response_receiver.recv_timeout(timeout){
                    Ok(r) => r,
                    Err(_) => {
                        if let Some(guard) = state.torrent_peers.try_lock(){
                            opt_torrent_peers = Some(guard);

                            break
                        } else {
                            continue
                        }
                    },
                }
            };

            responses.push(response);
        }

        let mut torrent_peers: MutexGuard<TorrentPeerMap> = opt_torrent_peers
            .unwrap_or_else(|| state.torrent_peers.lock());

        let requests = process_responses(
            &mut rng1,
            pareto,
            &state.info_hashes,
            config,
            &mut torrent_peers,
            responses.drain(..)
        );

        // Somewhat dubious heuristic for deciding how fast to create
        // and send additional requests (requests not having anything
        // to do with previously sent requests)
        let num_additional_to_send = {
            let num_additional_requests = requests.iter()
                .map(|v| v.len())
                .sum::<usize>() as f64;
            
            let num_new_requests_per_socket = num_additional_requests /
                config.num_socket_workers as f64;

            ((num_new_requests_per_socket / 1.2) * config.handler.additional_request_factor) as usize + 10
        };

        for (channel_index, new_requests) in requests.into_iter().enumerate(){
            let channel = &request_senders[channel_index];

            for _ in 0..num_additional_to_send {
                let request = create_connect_request(
                    generate_transaction_id(&mut rng2)
                );

                channel.send(request)
                    .expect("send request to channel in handler worker");
            }
            
            for request in new_requests.into_iter(){
                channel.send(request)
                    .expect("send request to channel in handler worker");
            }
        }
    }
}


fn process_responses(
    rng: &mut impl Rng,
    pareto: Pareto<f64>,
    info_hashes: &Arc<Vec<InfoHash>>,
    config: &Config,
    torrent_peers: &mut TorrentPeerMap,
    responses: Drain<(ThreadId, Response)>
) -> Vec<Vec<Request>> {
    let mut new_requests = Vec::with_capacity(
        config.num_socket_workers as usize
    );

    for _ in 0..config.num_socket_workers {
        new_requests.push(Vec::new());
    }

    for (socket_thread_id, response) in responses.into_iter() {
        let opt_request = process_response(
            rng,
            pareto,
            info_hashes,
            &config,
            torrent_peers,
            response
        );

        if let Some(new_request) = opt_request {
            new_requests[socket_thread_id.0 as usize].push(new_request);
        }
    }

    new_requests
}


fn process_response(
    rng: &mut impl Rng,
    pareto: Pareto<f64>,
    info_hashes: &Arc<Vec<InfoHash>>,
    config: &Config,
    torrent_peers: &mut TorrentPeerMap,
    response: Response
) -> Option<Request> {

    match response {
        Response::Connect(r) => {
            // Fetch the torrent peer or create it if is doesn't exists. Update
            // the connection id if fetched. Create a request and move the
            // torrent peer appropriately.

            let torrent_peer = torrent_peers.remove(&r.transaction_id)
                .map(|mut torrent_peer| {
                    torrent_peer.connection_id = r.connection_id;

                    torrent_peer
                })
                .unwrap_or_else(|| {
                    create_torrent_peer(
                        config,
                        rng,
                        pareto,
                        info_hashes,
                        r.connection_id
                    )
                });

            let new_transaction_id = generate_transaction_id(rng);

            let request = create_random_request(
                config,
                rng,
                info_hashes,
                new_transaction_id,
                &torrent_peer
            );

            torrent_peers.insert(new_transaction_id, torrent_peer);

            Some(request)

        },
        Response::Announce(r) => {
            if_torrent_peer_move_and_create_random_request(
                config,
                rng,
                info_hashes,
                torrent_peers,
                r.transaction_id
            )
        },
        Response::Scrape(r) => {
            if_torrent_peer_move_and_create_random_request(
                config,
                rng,
                info_hashes,
                torrent_peers,
                r.transaction_id
            )
        },
        Response::Error(r) => {
            if !r.message.to_lowercase().contains("connection"){
                eprintln!("Received error response which didn't contain the word 'connection': {}", r.message);
            }

            if let Some(torrent_peer) = torrent_peers.remove(&r.transaction_id){
                let new_transaction_id = generate_transaction_id(rng);

                torrent_peers.insert(new_transaction_id, torrent_peer);

                Some(create_connect_request(new_transaction_id))
            } else {
                Some(create_connect_request(generate_transaction_id(rng)))
            }
        }
    }
}


fn if_torrent_peer_move_and_create_random_request(
    config: &Config,
    rng: &mut impl Rng,
    info_hashes: &Arc<Vec<InfoHash>>,
    torrent_peers: &mut TorrentPeerMap,
    transaction_id: TransactionId,
) -> Option<Request> {
    if let Some(torrent_peer) = torrent_peers.remove(&transaction_id){
        let new_transaction_id = generate_transaction_id(rng);

        let request = create_random_request(
            config,
            rng,
            info_hashes,
            new_transaction_id,
            &torrent_peer
        );

        torrent_peers.insert(new_transaction_id, torrent_peer);

        Some(request)
    } else {
        None
    }
}


fn create_random_request(
    config: &Config,
    rng: &mut impl Rng,
    info_hashes: &Arc<Vec<InfoHash>>,
    transaction_id: TransactionId,
    torrent_peer: &TorrentPeer
) -> Request {
    let weights = vec![
        config.handler.weight_announce as u32,
        config.handler.weight_connect as u32, 
        config.handler.weight_scrape as u32, 
    ];

    let items = vec![
        RequestType::Announce,
        RequestType::Connect,
        RequestType::Scrape,
    ];

    let dist = WeightedIndex::new(&weights)
        .expect("random request weighted index");

    match items[dist.sample(rng)] {
        RequestType::Announce => create_announce_request(
            config,
            rng,
            torrent_peer,
            transaction_id
        ),
        RequestType::Connect => create_connect_request(transaction_id),
        RequestType::Scrape => create_scrape_request(
            &info_hashes,
            torrent_peer,
            transaction_id
        )
    }
}


fn create_announce_request(
    config: &Config,
    rng: &mut impl Rng,
    torrent_peer: &TorrentPeer,
    transaction_id: TransactionId,
) -> Request {
    let (event, bytes_left) = {
        if rng.gen_bool(config.handler.peer_seeder_probability) {
            (AnnounceEvent::Completed, NumberOfBytes(0))
        } else {
            (AnnounceEvent::Started, NumberOfBytes(50))
        }
    };

    (AnnounceRequest {
        connection_id: torrent_peer.connection_id,
        transaction_id,
        info_hash: torrent_peer.info_hash,
        peer_id: torrent_peer.peer_id,
        bytes_downloaded: NumberOfBytes(50),
        bytes_uploaded: NumberOfBytes(50),
        bytes_left,
        event,
        ip_address: None,
        key: PeerKey(12345),
        peers_wanted: NumberOfPeers(100),
        port: torrent_peer.port
    }).into()
}


fn create_scrape_request(
    info_hashes: &Arc<Vec<InfoHash>>,
    torrent_peer: &TorrentPeer,
    transaction_id: TransactionId,
) -> Request {
    let indeces = &torrent_peer.scrape_hash_indeces;

    let mut scape_hashes = Vec::with_capacity(indeces.len());

    for i in indeces {
        scape_hashes.push(info_hashes[*i].to_owned())
    }

    (ScrapeRequest {
        connection_id: torrent_peer.connection_id,
        transaction_id,
        info_hashes: scape_hashes,
    }).into()
}