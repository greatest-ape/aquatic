use std::net::SocketAddr;
use std::sync::atomic::Ordering;
use std::time::Instant;
use std::vec::Drain;

use rand::{self, SeedableRng, rngs::SmallRng, thread_rng};
use rand::seq::IteratorRandom;

use bittorrent_udp::types::*;

use crate::common::*;


pub fn handle_connect_requests(
    state: &State,
    responses: &mut Vec<(Response, SocketAddr)>,
    requests: Drain<(ConnectRequest, SocketAddr)>,
){
    let now = Time(Instant::now());

    for (request, src) in requests {
        let connection_id = ConnectionId(rand::random());

        let key = ConnectionKey {
            connection_id,
            socket_addr: src,
        };

        state.connections.insert(key, now);

        responses.push((Response::Connect(
            ConnectResponse {
                connection_id,
                transaction_id: request.transaction_id,
            }
        ), src));
    }
}


pub fn handle_announce_requests(
    state: &State,
    responses: &mut Vec<(Response, SocketAddr)>,
    requests: Drain<(AnnounceRequest, SocketAddr)>,
){
    for (request, src) in requests {
        let connection_key = ConnectionKey {
            connection_id: request.connection_id,
            socket_addr: src,
        };

        if !state.connections.contains_key(&connection_key){
            continue;
        }

        let mut torrent_data = state.torrents
            .entry(request.info_hash)
            .or_insert_with(|| TorrentData::default());

        let peer_key = PeerMapKey {
            ip: src.ip(),
            peer_id: request.peer_id,
        };

        let peer = Peer::from_announce_and_ip(&request, src.ip());
        let peer_status = peer.status;
        
        let opt_removed_peer = if peer.status == PeerStatus::Stopped {
            torrent_data.peers.remove(&peer_key)
        } else {
            torrent_data.peers.insert(peer_key, peer)
        };
        
        match peer_status {
            PeerStatus::Leeching => {
                torrent_data.num_leechers.fetch_add(1, Ordering::SeqCst);
            },
            PeerStatus::Seeding => {
                torrent_data.num_seeders.fetch_add(1, Ordering::SeqCst);
            },
            PeerStatus::Stopped => {}
        };

        if let Some(removed_peer) = opt_removed_peer {
            match removed_peer.status {
                PeerStatus::Leeching => {
                    torrent_data.num_leechers.fetch_sub(1, Ordering::SeqCst);
                },
                PeerStatus::Seeding => {
                    torrent_data.num_seeders.fetch_sub(1, Ordering::SeqCst);
                },
                PeerStatus::Stopped => {}
            }
        }

        let response_peers = extract_response_peers(&torrent_data.peers, 100); // FIXME num peers

        let response = Response::Announce(AnnounceResponse {
            transaction_id: request.transaction_id,
            announce_interval: AnnounceInterval(
                600 // config.announce_interval as i32
            ),
            leechers: NumberOfPeers(torrent_data.num_leechers.load(Ordering::SeqCst) as i32),
            seeders: NumberOfPeers(torrent_data.num_seeders.load(Ordering::SeqCst) as i32),
            peers: response_peers
        });

        responses.push((response, src));
    }
}


pub fn handle_scrape_requests(
    state: &State,
    responses: &mut Vec<(Response, SocketAddr)>,
    requests: Drain<(ScrapeRequest, SocketAddr)>,
){
    let empty_stats = create_torrent_scrape_statistics(0, 0);

    for (request, src) in requests {
        let mut stats: Vec<TorrentScrapeStatistics> = Vec::with_capacity(256);

        for info_hash in request.info_hashes.iter() {
            if let Some(torrent_data) = state.torrents.get(info_hash){
                stats.push(create_torrent_scrape_statistics(
                    torrent_data.num_seeders.load(Ordering::SeqCst) as i32,
                    torrent_data.num_leechers.load(Ordering::SeqCst) as i32,
                ));
            } else {
                stats.push(empty_stats);
            }
        }

        let response = Response::Scrape(ScrapeResponse {
            transaction_id: request.transaction_id,
            torrent_stats: stats,
        });

        responses.push((response, src));
    }
}


/// Extract response peers
/// 
/// Do a random selection of peers if there are more than
/// `number_of_peers_to_take`. I tried out just selecting a random range,
/// but this might cause issues with the announcing peer getting back too
/// homogenous peers (based on when they were inserted into the map.)
/// 
/// Don't care if we send back announcing peer.
pub fn extract_response_peers(
    peer_map:                &PeerMap,
    number_of_peers_to_take: usize,
) -> Vec<ResponsePeer> {
    let peer_map_len = peer_map.len();

    if peer_map_len <= number_of_peers_to_take {
        peer_map.values()
            .map(Peer::to_response_peer)
            .collect()
    } else {
        let mut rng = SmallRng::from_rng(thread_rng()).unwrap();

        peer_map.values()
            .map(Peer::to_response_peer)
            .choose_multiple(&mut rng, number_of_peers_to_take)
    }
}


pub fn create_torrent_scrape_statistics(
    seeders: i32,
    leechers: i32
) -> TorrentScrapeStatistics {
    TorrentScrapeStatistics {
        seeders: NumberOfPeers(seeders),
        completed: NumberOfDownloads(0), // No implementation planned
        leechers: NumberOfPeers(leechers)
    }
}