use std::net::SocketAddr;
use std::sync::atomic::Ordering;
use std::time::Instant;

use rand::{self, SeedableRng, rngs::SmallRng, thread_rng};
use rand::seq::IteratorRandom;

use bittorrent_udp::types::*;

use crate::types::*;


pub fn gen_responses(
    state: &State,
    connect_requests: Vec<(ConnectRequest, SocketAddr)>,
    announce_requests: Vec<(AnnounceRequest, SocketAddr)>
)-> Vec<(Response, SocketAddr)> {
    let mut responses = Vec::new();

    let now = Time(Instant::now());

    for (request, src) in connect_requests {
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

    for (request, src) in announce_requests {
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

    responses
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