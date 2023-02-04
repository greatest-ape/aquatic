mod storage;

use std::net::IpAddr;
use std::sync::atomic::Ordering;
use std::sync::Arc;
use std::time::Duration;
use std::time::Instant;

use aquatic_common::access_list::create_access_list_cache;
use aquatic_common::access_list::AccessListArcSwap;
use aquatic_common::ServerStartInstant;
use crossbeam_channel::Receiver;
use rand::Rng;
use rand::{rngs::SmallRng, SeedableRng};

use aquatic_common::{CanonicalSocketAddr, PanicSentinel, ValidUntil};

use aquatic_udp_protocol::*;

use crate::common::*;
use crate::config::Config;

use storage::ProtocolTorrentMaps;

use self::storage::TorrentMaps;

pub fn run_swarm_worker(
    _sentinel: PanicSentinel,
    config: Config,
    state: State,
    server_start_instant: ServerStartInstant,
    request_receiver: Receiver<(SocketWorkerIndex, ConnectedRequest, CanonicalSocketAddr)>,
    response_sender: ConnectedResponseSender,
    worker_index: SwarmWorkerIndex,
) {
    let mut torrents = ProtocolTorrentMaps::new(&config, server_start_instant);
    let mut rng = SmallRng::from_entropy();

    let timeout = Duration::from_millis(config.request_channel_recv_timeout_ms);
    let mut peer_valid_until = ValidUntil::new(server_start_instant, config.cleaning.max_peer_age);

    let cleaning_interval = Duration::from_secs(config.cleaning.torrent_cleaning_interval as u64);
    let statistics_update_interval = Duration::from_secs(config.statistics.interval);

    let mut last_cleaning = Instant::now();
    let mut last_statistics_update = Instant::now();

    let mut iter_counter = 0usize;

    loop {
        if let Ok((sender_index, request, src)) = request_receiver.recv_timeout(timeout) {
            let response = match (request, src.get().ip()) {
                (ConnectedRequest::Announce(request), IpAddr::V4(ip)) => {
                    let response = handle_announce_request(
                        &config,
                        &state.statistics_ipv4,
                        &state.access_list,
                        worker_index,
                        server_start_instant,
                        &mut rng,
                        &mut torrents.ipv4,
                        request,
                        ip,
                        peer_valid_until,
                    );

                    ConnectedResponse::AnnounceIpv4(response)
                }
                (ConnectedRequest::Announce(request), IpAddr::V6(ip)) => {
                    let response = handle_announce_request(
                        &config,
                        &state.statistics_ipv6,
                        &state.access_list,
                        worker_index,
                        server_start_instant,
                        &mut rng,
                        &mut torrents.ipv6,
                        request,
                        ip,
                        peer_valid_until,
                    );

                    ConnectedResponse::AnnounceIpv6(response)
                }
                (ConnectedRequest::Scrape(request), IpAddr::V4(_)) => {
                    ConnectedResponse::Scrape(handle_scrape_request(&mut torrents.ipv4, request))
                }
                (ConnectedRequest::Scrape(request), IpAddr::V6(_)) => {
                    ConnectedResponse::Scrape(handle_scrape_request(&mut torrents.ipv6, request))
                }
            };

            response_sender.try_send_to(sender_index, response, src);
        }

        // Run periodic tasks
        if iter_counter % 128 == 0 {
            let now = Instant::now();

            peer_valid_until = ValidUntil::new(server_start_instant, config.cleaning.max_peer_age);

            if now > last_cleaning + cleaning_interval {
                torrents.clean_and_update_peer_statistics(
                    &config,
                    &state,
                    worker_index,
                    server_start_instant,
                );

                last_cleaning = now;
            }
            if config.statistics.active()
                && now > last_statistics_update + statistics_update_interval
            {
                state.statistics_ipv4.torrents[worker_index.0]
                    .store(torrents.ipv4.total_num_torrents(), Ordering::Relaxed);
                state.statistics_ipv6.torrents[worker_index.0]
                    .store(torrents.ipv6.total_num_torrents(), Ordering::Relaxed);

                last_statistics_update = now;
            }
        }

        iter_counter = iter_counter.wrapping_add(1);
    }
}

fn handle_announce_request<I: Ip>(
    config: &Config,
    statistics: &Statistics,
    access_list: &Arc<AccessListArcSwap>,
    worker_index: SwarmWorkerIndex,
    server_start_instant: ServerStartInstant,
    rng: &mut SmallRng,
    torrent_maps: &mut TorrentMaps<I>,
    request: AnnounceRequest,
    peer_ip: I,
    peer_valid_until: ValidUntil,
) -> AnnounceResponse<I> {
    let torrent_map = torrent_maps.get_map(request.info_hash.0);

    if rng.gen_bool(config.cleaning.request_cleaning_probability) {
        let now = server_start_instant.seconds_elapsed();

        if torrent_map.needs_cleaning(now) {
            let mut access_list_cache = create_access_list_cache(access_list);

            let num_removed_peers = torrent_map.clean_and_get_num_removed_peers(
                config,
                &mut access_list_cache,
                config.access_list.mode,
                now,
            );

            statistics.peers[worker_index.0].fetch_sub(num_removed_peers, Ordering::Relaxed);

            ::log::info!(
                "Cleaned torrent map during announce request, removing {} peers",
                num_removed_peers
            );
        }
    }

    let torrent_data = torrent_map.torrents.entry(request.info_hash).or_default();

    let peer_status = PeerStatus::from_event_and_bytes_left(request.event, request.bytes_left);

    torrent_data.update_peer(
        config,
        statistics,
        worker_index,
        request.peer_id,
        peer_ip,
        request.port,
        peer_status,
        peer_valid_until,
    );

    let response_peers = if let PeerStatus::Stopped = peer_status {
        Vec::new()
    } else {
        let max_num_peers_to_take: usize = if request.peers_wanted.0 <= 0 {
            config.protocol.max_response_peers
        } else {
            ::std::cmp::min(
                config.protocol.max_response_peers,
                request.peers_wanted.0.try_into().unwrap(),
            )
        };

        torrent_data.extract_response_peers(rng, request.peer_id, max_num_peers_to_take)
    };

    AnnounceResponse {
        transaction_id: request.transaction_id,
        announce_interval: AnnounceInterval(config.protocol.peer_announce_interval),
        leechers: NumberOfPeers(torrent_data.num_leechers().try_into().unwrap_or(i32::MAX)),
        seeders: NumberOfPeers(torrent_data.num_seeders().try_into().unwrap_or(i32::MAX)),
        peers: response_peers,
    }
}

fn handle_scrape_request<I: Ip>(
    torrent_maps: &mut TorrentMaps<I>,
    request: PendingScrapeRequest,
) -> PendingScrapeResponse {
    const EMPTY_STATS: TorrentScrapeStatistics = create_torrent_scrape_statistics(0, 0);

    let torrent_stats = request
        .info_hashes
        .into_iter()
        .map(|(i, info_hash)| {
            let stats = torrent_maps
                .get_map(info_hash.0)
                .torrents
                .get(&info_hash)
                .map(|torrent_data| torrent_data.scrape_statistics())
                .unwrap_or(EMPTY_STATS);

            (i, stats)
        })
        .collect();

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
