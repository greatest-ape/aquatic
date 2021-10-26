use std::collections::BTreeMap;
use std::net::{IpAddr, Ipv4Addr, Ipv6Addr};

use either::Either;
use rand::{Rng};

use aquatic_common::extract_response_peers;
use aquatic_http_protocol::request::*;
use aquatic_http_protocol::response::*;

use crate::config::Config;
use super::*;

pub fn handle_announce_request(
    config: &Config,
    rng: &mut impl Rng,
    torrent_maps: &mut TorrentMaps,
    valid_until: ValidUntil,
    meta: ConnectionMeta,
    request: AnnounceRequest,
) -> AnnounceResponse {
    let peer_ip = convert_ipv4_mapped_ipv6(meta.peer_addr.ip());

    ::log::debug!("peer ip: {:?}", peer_ip);

    match peer_ip {
        IpAddr::V4(peer_ip_address) => {
            let torrent_data: &mut TorrentData<Ipv4Addr> =
                torrent_maps.ipv4.entry(request.info_hash).or_default();

            let peer_connection_meta = PeerConnectionMeta {
                worker_index: meta.worker_index,
                poll_token: meta.poll_token,
                peer_ip_address,
            };

            let (seeders, leechers, response_peers) = upsert_peer_and_get_response_peers(
                config,
                rng,
                peer_connection_meta,
                torrent_data,
                request,
                valid_until,
            );

            let response = AnnounceResponse {
                complete: seeders,
                incomplete: leechers,
                announce_interval: config.protocol.peer_announce_interval,
                peers: ResponsePeerListV4(response_peers),
                peers6: ResponsePeerListV6(vec![]),
            };

            response
        }
        IpAddr::V6(peer_ip_address) => {
            let torrent_data: &mut TorrentData<Ipv6Addr> =
                torrent_maps.ipv6.entry(request.info_hash).or_default();

            let peer_connection_meta = PeerConnectionMeta {
                worker_index: meta.worker_index,
                poll_token: meta.poll_token,
                peer_ip_address,
            };

            let (seeders, leechers, response_peers) = upsert_peer_and_get_response_peers(
                config,
                rng,
                peer_connection_meta,
                torrent_data,
                request,
                valid_until,
            );

            let response = AnnounceResponse {
                complete: seeders,
                incomplete: leechers,
                announce_interval: config.protocol.peer_announce_interval,
                peers: ResponsePeerListV4(vec![]),
                peers6: ResponsePeerListV6(response_peers),
            };

            response
        }
    }
}

/// Insert/update peer. Return num_seeders, num_leechers and response peers
pub fn upsert_peer_and_get_response_peers<I: Ip>(
    config: &Config,
    rng: &mut impl Rng,
    request_sender_meta: PeerConnectionMeta<I>,
    torrent_data: &mut TorrentData<I>,
    request: AnnounceRequest,
    valid_until: ValidUntil,
) -> (usize, usize, Vec<ResponsePeer<I>>) {
    // Insert/update/remove peer who sent this request

    let peer_status =
        PeerStatus::from_event_and_bytes_left(request.event, Some(request.bytes_left));

    let peer = Peer {
        connection_meta: request_sender_meta,
        port: request.port,
        status: peer_status,
        valid_until,
    };

    ::log::debug!("peer: {:?}", peer);

    let ip_or_key = request
        .key
        .map(Either::Right)
        .unwrap_or_else(|| Either::Left(request_sender_meta.peer_ip_address));

    let peer_map_key = PeerMapKey {
        peer_id: request.peer_id,
        ip_or_key,
    };

    ::log::debug!("peer map key: {:?}", peer_map_key);

    let opt_removed_peer = match peer_status {
        PeerStatus::Leeching => {
            torrent_data.num_leechers += 1;

            torrent_data.peers.insert(peer_map_key.clone(), peer)
        }
        PeerStatus::Seeding => {
            torrent_data.num_seeders += 1;

            torrent_data.peers.insert(peer_map_key.clone(), peer)
        }
        PeerStatus::Stopped => torrent_data.peers.remove(&peer_map_key),
    };

    ::log::debug!("opt_removed_peer: {:?}", opt_removed_peer);

    match opt_removed_peer.map(|peer| peer.status) {
        Some(PeerStatus::Leeching) => {
            torrent_data.num_leechers -= 1;
        }
        Some(PeerStatus::Seeding) => {
            torrent_data.num_seeders -= 1;
        }
        _ => {}
    }

    ::log::debug!("peer request numwant: {:?}", request.numwant);

    let max_num_peers_to_take = match request.numwant {
        Some(0) | None => config.protocol.max_peers,
        Some(numwant) => numwant.min(config.protocol.max_peers),
    };

    let response_peers: Vec<ResponsePeer<I>> = extract_response_peers(
        rng,
        &torrent_data.peers,
        max_num_peers_to_take,
        peer_map_key,
        Peer::to_response_peer,
    );

    (
        torrent_data.num_seeders,
        torrent_data.num_leechers,
        response_peers,
    )
}

pub fn handle_scrape_request(
    config: &Config,
    torrent_maps: &mut TorrentMaps,
    meta: ConnectionMeta,
    request: ScrapeRequest,
) -> ScrapeResponse {
    let num_to_take = request
        .info_hashes
        .len()
        .min(config.protocol.max_scrape_torrents);

    let mut response = ScrapeResponse {
        files: BTreeMap::new(),
    };

    let peer_ip = convert_ipv4_mapped_ipv6(meta.peer_addr.ip());

    // If request.info_hashes is empty, don't return scrape for all
    // torrents, even though reference server does it. It is too expensive.
    if peer_ip.is_ipv4() {
        for info_hash in request.info_hashes.into_iter().take(num_to_take) {
            if let Some(torrent_data) = torrent_maps.ipv4.get(&info_hash) {
                let stats = ScrapeStatistics {
                    complete: torrent_data.num_seeders,
                    downloaded: 0, // No implementation planned
                    incomplete: torrent_data.num_leechers,
                };

                response.files.insert(info_hash, stats);
            }
        }
    } else {
        for info_hash in request.info_hashes.into_iter().take(num_to_take) {
            if let Some(torrent_data) = torrent_maps.ipv6.get(&info_hash) {
                let stats = ScrapeStatistics {
                    complete: torrent_data.num_seeders,
                    downloaded: 0, // No implementation planned
                    incomplete: torrent_data.num_leechers,
                };

                response.files.insert(info_hash, stats);
            }
        }
    };

    response
}

