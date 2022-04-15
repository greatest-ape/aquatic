mod storage;

use std::collections::BTreeMap;
use std::net::IpAddr;
use std::sync::atomic::Ordering;
use std::time::Duration;
use std::time::Instant;

use aquatic_common::CanonicalSocketAddr;
use aquatic_common::PanicSentinel;
use aquatic_common::ValidUntil;
use crossbeam_channel::Receiver;
use rand::{rngs::SmallRng, SeedableRng};

use aquatic_common::extract_response_peers;

use aquatic_udp_protocol::*;

use crate::common::*;
use crate::config::Config;

use storage::{Peer, TorrentMap, TorrentMaps};

pub fn run_request_worker(
    _sentinel: PanicSentinel,
    config: Config,
    state: State,
    request_receiver: Receiver<(SocketWorkerIndex, ConnectedRequest, CanonicalSocketAddr)>,
    response_sender: ConnectedResponseSender,
    worker_index: RequestWorkerIndex,
) {
    let mut torrents = TorrentMaps::default();
    let mut rng = SmallRng::from_entropy();

    let timeout = Duration::from_millis(config.request_channel_recv_timeout_ms);
    let mut peer_valid_until = ValidUntil::new(config.cleaning.max_peer_age);

    let cleaning_interval = Duration::from_secs(config.cleaning.torrent_cleaning_interval);
    let statistics_update_interval = Duration::from_secs(config.statistics.interval);

    let mut last_cleaning = Instant::now();
    let mut last_statistics_update = Instant::now();

    let mut iter_counter = 0usize;

    loop {
        if let Ok((sender_index, request, src)) = request_receiver.recv_timeout(timeout) {
            let response = match request {
                ConnectedRequest::Announce(request) => match src.get().ip() {
                    IpAddr::V4(ip) => ConnectedResponse::AnnounceIpv4(handle_announce_request(
                        &config,
                        &mut rng,
                        &mut torrents.ipv4,
                        request,
                        ip,
                        peer_valid_until,
                    )),
                    IpAddr::V6(ip) => ConnectedResponse::AnnounceIpv6(handle_announce_request(
                        &config,
                        &mut rng,
                        &mut torrents.ipv6,
                        request,
                        ip,
                        peer_valid_until,
                    )),
                },
                ConnectedRequest::Scrape(request) => {
                    ConnectedResponse::Scrape(handle_scrape_request(&mut torrents, src, request))
                }
            };

            response_sender.try_send_to(sender_index, response, src);
        }

        if iter_counter % 128 == 0 {
            let now = Instant::now();

            peer_valid_until = ValidUntil::new_with_now(now, config.cleaning.max_peer_age);

            if now > last_cleaning + cleaning_interval {
                torrents.clean(&config, &state.access_list);

                if config.statistics.active() {
                    let peers_ipv4 = torrents.ipv4.values().map(|t| t.peers.len()).sum();
                    let peers_ipv6 = torrents.ipv6.values().map(|t| t.peers.len()).sum();

                    state.statistics_ipv4.peers[worker_index.0]
                        .store(peers_ipv4, Ordering::Release);
                    state.statistics_ipv6.peers[worker_index.0]
                        .store(peers_ipv6, Ordering::Release);
                }

                last_cleaning = now;
            }
            if config.statistics.active()
                && now > last_statistics_update + statistics_update_interval
            {
                state.statistics_ipv4.torrents[worker_index.0]
                    .store(torrents.ipv4.len(), Ordering::Release);
                state.statistics_ipv6.torrents[worker_index.0]
                    .store(torrents.ipv6.len(), Ordering::Release);

                last_statistics_update = now;
            }
        }

        iter_counter = iter_counter.wrapping_add(1);
    }
}

fn handle_announce_request<I: Ip>(
    config: &Config,
    rng: &mut SmallRng,
    torrents: &mut TorrentMap<I>,
    request: AnnounceRequest,
    peer_ip: I,
    peer_valid_until: ValidUntil,
) -> AnnounceResponse<I> {
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

            torrent_data.peers.insert(request.peer_id, peer)
        }
        PeerStatus::Seeding => {
            torrent_data.num_seeders += 1;

            torrent_data.peers.insert(request.peer_id, peer)
        }
        PeerStatus::Stopped => torrent_data.peers.remove(&request.peer_id),
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
        request.peer_id,
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

fn handle_scrape_request(
    torrents: &mut TorrentMaps,
    src: CanonicalSocketAddr,
    request: PendingScrapeRequest,
) -> PendingScrapeResponse {
    const EMPTY_STATS: TorrentScrapeStatistics = create_torrent_scrape_statistics(0, 0);

    let mut torrent_stats: BTreeMap<usize, TorrentScrapeStatistics> = BTreeMap::new();

    if src.is_ipv4() {
        torrent_stats.extend(request.info_hashes.into_iter().map(|(i, info_hash)| {
            let s = if let Some(torrent_data) = torrents.ipv4.get(&info_hash) {
                create_torrent_scrape_statistics(
                    torrent_data.num_seeders as i32,
                    torrent_data.num_leechers as i32,
                )
            } else {
                EMPTY_STATS
            };

            (i, s)
        }));
    } else {
        torrent_stats.extend(request.info_hashes.into_iter().map(|(i, info_hash)| {
            let s = if let Some(torrent_data) = torrents.ipv6.get(&info_hash) {
                create_torrent_scrape_statistics(
                    torrent_data.num_seeders as i32,
                    torrent_data.num_leechers as i32,
                )
            } else {
                EMPTY_STATS
            };

            (i, s)
        }));
    }

    PendingScrapeResponse {
        slab_key: request.slab_key,
        torrent_stats,
    }
}

#[inline(always)]
const fn create_torrent_scrape_statistics(seeders: i32, leechers: i32) -> TorrentScrapeStatistics {
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

    use quickcheck::{quickcheck, TestResult};
    use rand::thread_rng;

    use super::*;

    fn gen_peer_id(i: u32) -> PeerId {
        let mut peer_id = PeerId([0; 20]);

        peer_id.0[0..4].copy_from_slice(&i.to_ne_bytes());

        peer_id
    }
    fn gen_peer(i: u32) -> Peer<Ipv4Addr> {
        Peer {
            ip_address: Ipv4Addr::from(i.to_be_bytes()),
            port: Port(1),
            status: PeerStatus::Leeching,
            valid_until: ValidUntil::new(0),
        }
    }

    #[test]
    fn test_extract_response_peers() {
        fn prop(data: (u16, u16)) -> TestResult {
            let gen_num_peers = data.0 as u32;
            let req_num_peers = data.1 as usize;

            let mut peer_map: storage::PeerMap<Ipv4Addr> = Default::default();

            let mut opt_sender_key = None;
            let mut opt_sender_peer = None;

            for i in 0..gen_num_peers {
                let key = gen_peer_id(i);
                let peer = gen_peer((i << 16) + i);

                if i == 0 {
                    opt_sender_key = Some(key);
                    opt_sender_peer = Some(peer.to_response_peer());
                }

                peer_map.insert(key, peer);
            }

            let mut rng = thread_rng();

            let peers = extract_response_peers(
                &mut rng,
                &peer_map,
                req_num_peers,
                opt_sender_key.unwrap_or_else(|| gen_peer_id(1)),
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
