use std::time::Duration;
use std::vec::Drain;

use hashbrown::HashMap;
use parking_lot::MutexGuard;
use rand::{Rng, SeedableRng, rngs::SmallRng};

use aquatic_common::extract_response_peers;

use crate::common::*;
use crate::config::Config;

use crate::protocol::request::*;
use crate::protocol::response::*;


// almost identical to ws version
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

    responses.extend(requests.into_iter().map(|(request_sender_meta, request)| {
        let torrent_data: &mut TorrentData = if request_sender_meta.peer_addr.is_ipv4(){
            torrent_maps.ipv4.entry(request.info_hash).or_default()
        } else {
            torrent_maps.ipv6.entry(request.info_hash).or_default()
        };

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

            let opt_removed_peer = match peer_status {
                PeerStatus::Leeching => {
                    torrent_data.num_leechers += 1;

                    torrent_data.peers.insert(request.peer_id, peer)
                },
                PeerStatus::Seeding => {
                    torrent_data.num_seeders += 1;

                    torrent_data.peers.insert(request.peer_id, peer)
                },
                PeerStatus::Stopped => {
                    torrent_data.peers.remove(&request.peer_id)
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

        let response_peers: Vec<ResponsePeer> = extract_response_peers(
            rng,
            &torrent_data.peers,
            max_num_peers_to_take,
            ResponsePeer::from_peer
        );

        let response = Response::Announce(AnnounceResponse {
            complete: torrent_data.num_seeders,
            incomplete: torrent_data.num_leechers,
            announce_interval: config.protocol.peer_announce_interval,
            peers: response_peers,
        });

        (request_sender_meta, response)
    }));
}


// almost identical to ws version
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

        let torrent_map: &mut TorrentMap = if meta.peer_addr.is_ipv4(){
            &mut torrent_maps.ipv4
        } else {
            &mut torrent_maps.ipv6
        };

        // If request.info_hashes is empty, don't return scrape for all
        // torrents, even though reference server does it. It is too expensive.
        for info_hash in request.info_hashes.into_iter().take(num_to_take){
            if let Some(torrent_data) = torrent_map.get(&info_hash){
                let stats = ScrapeStatistics {
                    complete: torrent_data.num_seeders,
                    downloaded: 0, // No implementation planned
                    incomplete: torrent_data.num_leechers,
                };

                response.files.insert(info_hash, stats);
            }
        }

        (meta, Response::Scrape(response))
    }));
}