use std::time::Duration;
use std::vec::Drain;
use std::net::{IpAddr, Ipv4Addr, Ipv6Addr};

use either::Either;
use hashbrown::HashMap;
use parking_lot::MutexGuard;
use rand::{Rng, SeedableRng, rngs::SmallRng};

use aquatic_common::extract_response_peers;

use crate::common::*;
use crate::config::Config;

use crate::protocol::request::*;
use crate::protocol::response::*;


pub fn run_request_worker(
    config: Config,
    state: State,
    request_channel_receiver: RequestChannelReceiver,
    response_channel_sender: ResponseChannelSender,
){
    let mut responses = Vec::new();

    let mut announce_requests = Vec::new();
    let mut scrape_requests = Vec::new();

    let mut rng = SmallRng::from_entropy();

    let timeout = Duration::from_micros(
        config.handlers.channel_recv_timeout_microseconds
    );

    loop {
        let mut opt_torrent_map_guard: Option<MutexGuard<TorrentMaps>> = None;

        for i in 0..config.handlers.max_requests_per_iter {
            let opt_in_message = if i == 0 {
                request_channel_receiver.recv().ok()
            } else {
                request_channel_receiver.recv_timeout(timeout).ok()
            };

            match opt_in_message {
                Some((meta, Request::Announce(r))) => {
                    announce_requests.push((meta, r));
                },
                Some((meta, Request::Scrape(r))) => {
                    scrape_requests.push((meta, r));
                },
                None => {
                    if let Some(torrent_guard) = state.torrent_maps.try_lock(){
                        opt_torrent_map_guard = Some(torrent_guard);

                        break
                    }
                }
            }
        }

        let mut torrent_map_guard = opt_torrent_map_guard
            .unwrap_or_else(|| state.torrent_maps.lock());

        handle_announce_requests(
            &config,
            &mut rng,
            &mut torrent_map_guard,
            &mut responses,
            announce_requests.drain(..)
        );

        handle_scrape_requests(
            &config,
            &mut torrent_map_guard,
            &mut responses,
            scrape_requests.drain(..)
        );

        ::std::mem::drop(torrent_map_guard);

        for (meta, response) in responses.drain(..){
            response_channel_sender.send(meta, response);
        }
    }
}


pub fn handle_announce_requests(
    config: &Config,
    rng: &mut impl Rng,
    torrent_maps: &mut TorrentMaps,
    responses: &mut Vec<(ConnectionMeta, Response)>,
    requests: Drain<(ConnectionMeta, AnnounceRequest)>,
){
    let valid_until = ValidUntil::new(config.cleaning.max_peer_age);

    responses.extend(requests.map(|(request_sender_meta, request)| {
        let peer_ip = convert_ipv4_mapped_ipv4(
            request_sender_meta.peer_addr.ip()
        );

        let response = match peer_ip {
            IpAddr::V4(peer_ip_address) => {
                let torrent_data: &mut TorrentData<Ipv4Addr> = torrent_maps.ipv4
                    .entry(request.info_hash)
                    .or_default();
                
                let peer_connection_meta = PeerConnectionMeta {
                    worker_index: request_sender_meta.worker_index,
                    poll_token: request_sender_meta.poll_token,
                    peer_ip_address,
                };

                let (seeders, leechers, response_peers) = upsert_peer_and_get_response_peers(
                    config,
                    rng,
                    peer_connection_meta,
                    torrent_data,
                    request,
                    valid_until
                );

                let response = AnnounceResponse {
                    complete: seeders,
                    incomplete: leechers,
                    announce_interval: config.protocol.peer_announce_interval,
                    peers: ResponsePeerListV4(response_peers),
                    peers6: ResponsePeerListV6(vec![]),
                };

                Response::Announce(response)
            },
            IpAddr::V6(peer_ip_address) => {
                let torrent_data: &mut TorrentData<Ipv6Addr> = torrent_maps.ipv6
                    .entry(request.info_hash)
                    .or_default();
                
                let peer_connection_meta = PeerConnectionMeta {
                    worker_index: request_sender_meta.worker_index,
                    poll_token: request_sender_meta.poll_token,
                    peer_ip_address
                };

                let (seeders, leechers, response_peers) = upsert_peer_and_get_response_peers(
                    config,
                    rng,
                    peer_connection_meta,
                    torrent_data,
                    request,
                    valid_until
                );

                let response = AnnounceResponse {
                    complete: seeders,
                    incomplete: leechers,
                    announce_interval: config.protocol.peer_announce_interval,
                    peers: ResponsePeerListV4(vec![]),
                    peers6: ResponsePeerListV6(response_peers),
                };

                Response::Announce(response)
            },
        };

        (request_sender_meta, response)
    }));
}


/// Insert/update peer. Return num_seeders, num_leechers and response peers
fn upsert_peer_and_get_response_peers<I: Ip>(
    config: &Config,
    rng: &mut impl Rng,
    request_sender_meta: PeerConnectionMeta<I>,
    torrent_data: &mut TorrentData<I>,
    request: AnnounceRequest,
    valid_until: ValidUntil,
) -> (usize, usize, Vec<ResponsePeer<I>>) {
    // Insert/update/remove peer who sent this request
    {
        let peer_status = PeerStatus::from_event_and_bytes_left(
            request.event,
            Some(request.bytes_left)
        );

        let peer = Peer {
            connection_meta: request_sender_meta,
            port: request.port,
            status: peer_status,
            valid_until,
        };

        let ip_or_key = request.key
            .map(Either::Right)
            .unwrap_or_else(||
                Either::Left(request_sender_meta.peer_ip_address)
            );

        let peer_map_key = PeerMapKey {
            peer_id: request.peer_id,
            ip_or_key,
        };

        let opt_removed_peer = match peer_status {
            PeerStatus::Leeching => {
                torrent_data.num_leechers += 1;

                torrent_data.peers.insert(peer_map_key, peer)
            },
            PeerStatus::Seeding => {
                torrent_data.num_seeders += 1;

                torrent_data.peers.insert(peer_map_key, peer)
            },
            PeerStatus::Stopped => {
                torrent_data.peers.remove(&peer_map_key)
            }
        };

        match opt_removed_peer.map(|peer| peer.status){
            Some(PeerStatus::Leeching) => {
                torrent_data.num_leechers -= 1;
            },
            Some(PeerStatus::Seeding) => {
                torrent_data.num_seeders -= 1;
            },
            _ => {}
        }
    }

    let max_num_peers_to_take = match request.numwant {
        Some(0) | None => config.protocol.max_peers,
        Some(numwant) => numwant.min(config.protocol.max_peers),
    };

    let response_peers: Vec<ResponsePeer<I>> = extract_response_peers(
        rng,
        &torrent_data.peers,
        max_num_peers_to_take,
        Peer::to_response_peer
    );

    (torrent_data.num_seeders, torrent_data.num_leechers, response_peers)
}


pub fn handle_scrape_requests(
    config: &Config,
    torrent_maps: &mut TorrentMaps,
    messages_out: &mut Vec<(ConnectionMeta, Response)>,
    requests: Drain<(ConnectionMeta, ScrapeRequest)>,
){
    messages_out.extend(requests.map(|(meta, request)| {
        let num_to_take = request.info_hashes.len().min(
            config.protocol.max_scrape_torrents
        );

        let mut response = ScrapeResponse {
            files: HashMap::with_capacity(num_to_take),
        };

        let peer_ip = convert_ipv4_mapped_ipv4(
            meta.peer_addr.ip()
        );

        // If request.info_hashes is empty, don't return scrape for all
        // torrents, even though reference server does it. It is too expensive.
        if peer_ip.is_ipv4(){
            for info_hash in request.info_hashes.into_iter().take(num_to_take){
                if let Some(torrent_data) = torrent_maps.ipv4.get(&info_hash){
                    let stats = ScrapeStatistics {
                        complete: torrent_data.num_seeders,
                        downloaded: 0, // No implementation planned
                        incomplete: torrent_data.num_leechers,
                    };

                    response.files.insert(info_hash, stats);
                }
            }
        } else {
            for info_hash in request.info_hashes.into_iter().take(num_to_take){
                if let Some(torrent_data) = torrent_maps.ipv6.get(&info_hash){
                    let stats = ScrapeStatistics {
                        complete: torrent_data.num_seeders,
                        downloaded: 0, // No implementation planned
                        incomplete: torrent_data.num_leechers,
                    };

                    response.files.insert(info_hash, stats);
                }
            }
        };


        (meta, Response::Scrape(response))
    }));
}