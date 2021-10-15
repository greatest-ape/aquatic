use std::net::{IpAddr, SocketAddr};
use std::time::Duration;
use std::vec::Drain;

use crossbeam_channel::{Receiver, Sender};
use parking_lot::MutexGuard;
use rand::{
    rngs::{SmallRng, StdRng},
    Rng, SeedableRng,
};

use aquatic_common::{convert_ipv4_mapped_ipv6, extract_response_peers, access_list::AccessListType};
use aquatic_udp_protocol::*;

use crate::common::*;
use crate::config::Config;

pub fn run_request_worker(
    state: State,
    config: Config,
    request_receiver: Receiver<(Request, SocketAddr)>,
    response_sender: Sender<(Response, SocketAddr)>,
) {
    let mut connect_requests: Vec<(ConnectRequest, SocketAddr)> = Vec::new();
    let mut announce_requests: Vec<(AnnounceRequest, SocketAddr)> = Vec::new();
    let mut scrape_requests: Vec<(ScrapeRequest, SocketAddr)> = Vec::new();

    let mut responses: Vec<(Response, SocketAddr)> = Vec::new();

    let mut std_rng = StdRng::from_entropy();
    let mut small_rng = SmallRng::from_rng(&mut std_rng).unwrap();

    let timeout = Duration::from_micros(config.handlers.channel_recv_timeout_microseconds);

    loop {
        let mut opt_connections = None;

        // Collect requests from channel, divide them by type
        //
        // Collect a maximum number of request. Stop collecting before that
        // number is reached if having waited for too long for a request, but
        // only if ConnectionMap mutex isn't locked.
        for i in 0..config.handlers.max_requests_per_iter {
            let (request, src): (Request, SocketAddr) = if i == 0 {
                match request_receiver.recv() {
                    Ok(r) => r,
                    Err(_) => break, // Really shouldn't happen
                }
            } else {
                match request_receiver.recv_timeout(timeout) {
                    Ok(r) => r,
                    Err(_) => {
                        if let Some(guard) = state.connections.try_lock() {
                            opt_connections = Some(guard);

                            break;
                        } else {
                            continue;
                        }
                    }
                }
            };

            match request {
                Request::Connect(r) => connect_requests.push((r, src)),
                Request::Announce(r) => announce_requests.push((r, src)),
                Request::Scrape(r) => scrape_requests.push((r, src)),
            }
        }

        let mut connections: MutexGuard<ConnectionMap> =
            opt_connections.unwrap_or_else(|| state.connections.lock());

        handle_connect_requests(
            &config,
            &mut connections,
            &mut std_rng,
            connect_requests.drain(..),
            &mut responses,
        );

        // Check announce and scrape requests for valid connection

        announce_requests.retain(|(request, src)| {
            let connection_key = ConnectionKey {
                connection_id: request.connection_id,
                socket_addr: *src,
            };

            if connections.contains_key(&connection_key) {
                true
            } else {
                let response = ErrorResponse {
                    transaction_id: request.transaction_id,
                    message: "Connection invalid or expired".to_string(),
                };

                responses.push((response.into(), *src));

                return false;
            }
        });

        scrape_requests.retain(|(request, src)| {
            let connection_key = ConnectionKey {
                connection_id: request.connection_id,
                socket_addr: *src,
            };

            if connections.contains_key(&connection_key) {
                true
            } else {
                let response = ErrorResponse {
                    transaction_id: request.transaction_id,
                    message: "Connection invalid or expired".to_string(),
                };

                responses.push((response.into(), *src));

                false
            }
        });

        ::std::mem::drop(connections);

        // Check announce requests for allowed info hashes

        match config.access_list.list_type {
            access_list_type @ (AccessListType::Allow | AccessListType::Deny) => {
                let access_list: MutexGuard<AccessList> = state.access_list.lock();

                announce_requests.retain(|(request, src)| {
                    if !access_list.allows(access_list_type, &request.info_hash.0) {
                        let response = ErrorResponse {
                            transaction_id: request.transaction_id,
                            message: "Info hash not allowed".to_string(),
                        };

                        responses.push((response.into(), *src));

                        return false;
                    }

                    true
                });
            }
            AccessListType::Ignore => {}
        };

        // Handle announce and scrape requests

        if !(announce_requests.is_empty() && scrape_requests.is_empty()) {
            let mut torrents = state.torrents.lock();

            handle_announce_requests(
                &config,
                &mut torrents,
                &mut small_rng,
                announce_requests.drain(..),
                &mut responses,
            );
            handle_scrape_requests(&mut torrents, scrape_requests.drain(..), &mut responses);
        }

        for r in responses.drain(..) {
            if let Err(err) = response_sender.send(r) {
                ::log::error!("error sending response to channel: {}", err);
            }
        }
    }
}

#[inline]
pub fn handle_connect_requests(
    config: &Config,
    connections: &mut MutexGuard<ConnectionMap>,
    rng: &mut StdRng,
    requests: Drain<(ConnectRequest, SocketAddr)>,
    responses: &mut Vec<(Response, SocketAddr)>,
) {
    let valid_until = ValidUntil::new(config.cleaning.max_connection_age);

    responses.extend(requests.map(|(request, src)| {
        let connection_id = ConnectionId(rng.gen());

        let key = ConnectionKey {
            connection_id,
            socket_addr: src,
        };

        connections.insert(key, valid_until);

        let response = Response::Connect(ConnectResponse {
            connection_id,
            transaction_id: request.transaction_id,
        });

        (response, src)
    }));
}

#[inline]
pub fn handle_announce_requests(
    config: &Config,
    torrents: &mut MutexGuard<TorrentMaps>,
    rng: &mut SmallRng,
    requests: Drain<(AnnounceRequest, SocketAddr)>,
    responses: &mut Vec<(Response, SocketAddr)>,
) {
    let peer_valid_until = ValidUntil::new(config.cleaning.max_peer_age);

    responses.extend(requests.map(|(request, src)| {
        let peer_ip = convert_ipv4_mapped_ipv6(src.ip());

        let response = match peer_ip {
            IpAddr::V4(ip) => handle_announce_request(
                config,
                rng,
                &mut torrents.ipv4,
                request,
                ip,
                peer_valid_until,
            ),
            IpAddr::V6(ip) => handle_announce_request(
                config,
                rng,
                &mut torrents.ipv6,
                request,
                ip,
                peer_valid_until,
            ),
        };

        (response.into(), src)
    }));
}

fn handle_announce_request<I: Ip>(
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
pub fn handle_scrape_requests(
    torrents: &mut MutexGuard<TorrentMaps>,
    requests: Drain<(ScrapeRequest, SocketAddr)>,
    responses: &mut Vec<(Response, SocketAddr)>,
) {
    let empty_stats = create_torrent_scrape_statistics(0, 0);

    responses.extend(requests.map(|(request, src)| {
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

        let response = Response::Scrape(ScrapeResponse {
            transaction_id: request.transaction_id,
            torrent_stats: stats,
        });

        (response, src)
    }));
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

#[inline(always)]
pub fn create_torrent_scrape_statistics(seeders: i32, leechers: i32) -> TorrentScrapeStatistics {
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
