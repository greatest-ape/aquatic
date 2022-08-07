mod storage;

use std::net::IpAddr;
use std::sync::atomic::Ordering;
use std::time::Duration;
use std::time::Instant;

use aquatic_common::ServerStartInstant;
use crossbeam_channel::Receiver;
use crossbeam_channel::Sender;
use rand::{rngs::SmallRng, SeedableRng};

use aquatic_common::{CanonicalSocketAddr, PanicSentinel, ValidUntil};

use aquatic_udp_protocol::*;

use crate::common::*;
use crate::config::Config;

use storage::{TorrentMap, TorrentMaps};

pub fn run_swarm_worker(
    _sentinel: PanicSentinel,
    config: Config,
    state: State,
    server_start_instant: ServerStartInstant,
    request_receiver: Receiver<(SocketWorkerIndex, ConnectedRequest, CanonicalSocketAddr)>,
    response_sender: ConnectedResponseSender,
    statistics_sender: Sender<StatisticsMessage>,
    worker_index: SwarmWorkerIndex,
) {
    let mut torrents = TorrentMaps::default();
    let mut rng = SmallRng::from_entropy();

    let timeout = Duration::from_millis(config.request_channel_recv_timeout_ms);
    let mut peer_valid_until = ValidUntil::new(server_start_instant, config.cleaning.max_peer_age);

    let cleaning_interval = Duration::from_secs(config.cleaning.torrent_cleaning_interval);
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
                let (ipv4, ipv6) = torrents.clean_and_get_statistics(
                    &config,
                    &state.access_list,
                    server_start_instant,
                );

                if config.statistics.active() {
                    state.statistics_ipv4.peers[worker_index.0].store(ipv4.0, Ordering::Release);
                    state.statistics_ipv6.peers[worker_index.0].store(ipv6.0, Ordering::Release);

                    if let Some(message) = ipv4.1.map(StatisticsMessage::Ipv4PeerHistogram) {
                        if let Err(err) = statistics_sender.try_send(message) {
                            ::log::error!("couldn't send statistics message: {:#}", err)
                        }
                    }
                    if let Some(message) = ipv6.1.map(StatisticsMessage::Ipv6PeerHistogram) {
                        if let Err(err) = statistics_sender.try_send(message) {
                            ::log::error!("couldn't send statistics message: {:#}", err)
                        }
                    }
                }

                last_cleaning = now;
            }
            if config.statistics.active()
                && now > last_statistics_update + statistics_update_interval
            {
                state.statistics_ipv4.torrents[worker_index.0]
                    .store(torrents.ipv4.num_torrents(), Ordering::Release);
                state.statistics_ipv6.torrents[worker_index.0]
                    .store(torrents.ipv6.num_torrents(), Ordering::Release);

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
    let max_num_peers_to_take = if request.peers_wanted.0 <= 0 {
        config.protocol.max_response_peers as usize
    } else {
        ::std::cmp::min(
            config.protocol.max_response_peers as usize,
            request.peers_wanted.0.try_into().unwrap(),
        )
    };

    let torrent_data = torrents.0.entry(request.info_hash).or_default();

    let peer_status = PeerStatus::from_event_and_bytes_left(request.event, request.bytes_left);

    torrent_data.update_peer(
        request.peer_id,
        peer_ip,
        request.port,
        peer_status,
        peer_valid_until,
    );

    let response_peers =
        torrent_data.extract_response_peers(rng, request.peer_id, max_num_peers_to_take);

    AnnounceResponse {
        transaction_id: request.transaction_id,
        announce_interval: AnnounceInterval(config.protocol.peer_announce_interval),
        leechers: NumberOfPeers(torrent_data.num_leechers() as i32),
        seeders: NumberOfPeers(torrent_data.num_seeders() as i32),
        peers: response_peers,
    }
}

fn handle_scrape_request<I: Ip>(
    torrents: &mut TorrentMap<I>,
    request: PendingScrapeRequest,
) -> PendingScrapeResponse {
    const EMPTY_STATS: TorrentScrapeStatistics = create_torrent_scrape_statistics(0, 0);

    let torrent_stats = request
        .info_hashes
        .into_iter()
        .map(|(i, info_hash)| {
            let stats = torrents
                .0
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
