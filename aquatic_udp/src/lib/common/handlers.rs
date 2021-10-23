use std::net::SocketAddr;

use rand::rngs::SmallRng;

use aquatic_common::extract_response_peers;
use aquatic_common::convert_ipv4_mapped_ipv6;

use crate::common::*;

#[derive(Debug)]
pub enum ConnectedRequest {
    Announce(AnnounceRequest),
    Scrape(ScrapeRequest),
}

#[derive(Debug)]
pub enum ConnectedResponse {
    Announce(AnnounceResponse),
    Scrape(ScrapeResponse),
}

impl Into<Response> for ConnectedResponse {
    fn into(self) -> Response {
        match self {
            Self::Announce(response) => Response::Announce(response),
            Self::Scrape(response) => Response::Scrape(response),
        }
    }
}

pub fn handle_announce_request<I: Ip>(
    config: &Config,
    rng: &mut SmallRng,
    torrents: &mut TorrentMap<I>,
    request: AnnounceRequest,
    peer_ip: I,
    peer_valid_until: ValidUntil,
) -> AnnounceResponse {
    let peer_key = PeerMapKey {
        ip: peer_ip,
        peer_id: request.peer_id,
    };

    let peer_status = PeerStatus::from_event_and_bytes_left(request.event, request.bytes_left);

    let peer = Peer {
        ip_address: peer_ip,
        port: request.port,
        status: peer_status,
        valid_until: peer_valid_until,
    };

    let torrent_data = torrents.entry(request.info_hash).or_default();

    let opt_removed_peer = match peer_status {
        PeerStatus::Leeching => {
            torrent_data.num_leechers += 1;

            torrent_data.peers.insert(peer_key, peer)
        }
        PeerStatus::Seeding => {
            torrent_data.num_seeders += 1;

            torrent_data.peers.insert(peer_key, peer)
        }
        PeerStatus::Stopped => torrent_data.peers.remove(&peer_key),
    };

    match opt_removed_peer.map(|peer| peer.status) {
        Some(PeerStatus::Leeching) => {
            torrent_data.num_leechers -= 1;
        }
        Some(PeerStatus::Seeding) => {
            torrent_data.num_seeders -= 1;
        }
        _ => {}
    }

    let max_num_peers_to_take = calc_max_num_peers_to_take(config, request.peers_wanted.0);

    let response_peers = extract_response_peers(
        rng,
        &torrent_data.peers,
        max_num_peers_to_take,
        peer_key,
        Peer::to_response_peer,
    );

    AnnounceResponse {
        transaction_id: request.transaction_id,
        announce_interval: AnnounceInterval(config.protocol.peer_announce_interval),
        leechers: NumberOfPeers(torrent_data.num_leechers as i32),
        seeders: NumberOfPeers(torrent_data.num_seeders as i32),
        peers: response_peers,
    }
}

#[inline]
fn calc_max_num_peers_to_take(config: &Config, peers_wanted: i32) -> usize {
    if peers_wanted <= 0 {
        config.protocol.max_response_peers as usize
    } else {
        ::std::cmp::min(
            config.protocol.max_response_peers as usize,
            peers_wanted as usize,
        )
    }
}

#[inline]
pub fn handle_scrape_request(
    torrents: &mut TorrentMaps,
    src: SocketAddr,
    request: ScrapeRequest,
) -> ScrapeResponse {
    let empty_stats = create_torrent_scrape_statistics(0, 0);

    let mut stats: Vec<TorrentScrapeStatistics> = Vec::with_capacity(request.info_hashes.len());

    let peer_ip = convert_ipv4_mapped_ipv6(src.ip());

    if peer_ip.is_ipv4() {
        for info_hash in request.info_hashes.iter() {
            if let Some(torrent_data) = torrents.ipv4.get(info_hash) {
                stats.push(create_torrent_scrape_statistics(
                    torrent_data.num_seeders as i32,
                    torrent_data.num_leechers as i32,
                ));
            } else {
                stats.push(empty_stats);
            }
        }
    } else {
        for info_hash in request.info_hashes.iter() {
            if let Some(torrent_data) = torrents.ipv6.get(info_hash) {
                stats.push(create_torrent_scrape_statistics(
                    torrent_data.num_seeders as i32,
                    torrent_data.num_leechers as i32,
                ));
            } else {
                stats.push(empty_stats);
            }
        }
    }

    ScrapeResponse {
        transaction_id: request.transaction_id,
        torrent_stats: stats,
    }
}

#[inline(always)]
fn create_torrent_scrape_statistics(seeders: i32, leechers: i32) -> TorrentScrapeStatistics {
    TorrentScrapeStatistics {
        seeders: NumberOfPeers(seeders),
        completed: NumberOfDownloads(0), // No implementation planned
        leechers: NumberOfPeers(leechers),
    }
}

#[cfg(test)]
mod tests {
    use std::collections::HashSet;
    use std::net::Ipv4Addr;

    use indexmap::IndexMap;
    use quickcheck::{quickcheck, TestResult};
    use rand::thread_rng;

    use super::*;

    fn gen_peer_map_key_and_value(i: u32) -> (PeerMapKey<Ipv4Addr>, Peer<Ipv4Addr>) {
        let ip_address = Ipv4Addr::from(i.to_be_bytes());
        let peer_id = PeerId([0; 20]);

        let key = PeerMapKey {
            ip: ip_address,
            peer_id,
        };
        let value = Peer {
            ip_address,
            port: Port(1),
            status: PeerStatus::Leeching,
            valid_until: ValidUntil::new(0),
        };

        (key, value)
    }

    #[test]
    fn test_extract_response_peers() {
        fn prop(data: (u16, u16)) -> TestResult {
            let gen_num_peers = data.0 as u32;
            let req_num_peers = data.1 as usize;

            let mut peer_map: PeerMap<Ipv4Addr> = IndexMap::with_capacity(gen_num_peers as usize);

            let mut opt_sender_key = None;
            let mut opt_sender_peer = None;

            for i in 0..gen_num_peers {
                let (key, value) = gen_peer_map_key_and_value((i << 16) + i);

                if i == 0 {
                    opt_sender_key = Some(key);
                    opt_sender_peer = Some(value.to_response_peer());
                }

                peer_map.insert(key, value);
            }

            let mut rng = thread_rng();

            let peers = extract_response_peers(
                &mut rng,
                &peer_map,
                req_num_peers,
                opt_sender_key.unwrap_or_else(|| gen_peer_map_key_and_value(1).0),
                Peer::to_response_peer,
            );

            // Check that number of returned peers is correct

            let mut success = peers.len() <= req_num_peers;

            if req_num_peers >= gen_num_peers as usize {
                success &= peers.len() == gen_num_peers as usize
                    || peers.len() + 1 == gen_num_peers as usize;
            }

            // Check that returned peers are unique (no overlap) and that sender
            // isn't returned

            let mut ip_addresses = HashSet::with_capacity(peers.len());

            for peer in peers {
                if peer == opt_sender_peer.clone().unwrap()
                    || ip_addresses.contains(&peer.ip_address)
                {
                    success = false;

                    break;
                }

                ip_addresses.insert(peer.ip_address);
            }

            TestResult::from_bool(success)
        }

        quickcheck(prop as fn((u16, u16)) -> TestResult);
    }
}
