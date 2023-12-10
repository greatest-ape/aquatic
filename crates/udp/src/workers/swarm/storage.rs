use std::sync::atomic::Ordering;
use std::sync::Arc;

use aquatic_common::IndexMap;
use aquatic_common::SecondsSinceServerStart;
use aquatic_common::ServerStartInstant;
use aquatic_common::{
    access_list::{create_access_list_cache, AccessListArcSwap, AccessListCache, AccessListMode},
    ValidUntil,
};

use aquatic_udp_protocol::*;
use crossbeam_channel::Sender;
use hdrhistogram::Histogram;
use rand::prelude::SmallRng;
use rand::Rng;

use crate::common::*;
use crate::config::Config;

#[derive(Clone, Debug)]
struct Peer<I: Ip> {
    ip_address: I,
    port: Port,
    is_seeder: bool,
    valid_until: ValidUntil,
}

impl<I: Ip> Peer<I> {
    fn to_response_peer(_: &PeerId, peer: &Self) -> ResponsePeer<I> {
        ResponsePeer {
            ip_address: peer.ip_address,
            port: peer.port,
        }
    }
}

type PeerMap<I> = IndexMap<PeerId, Peer<I>>;

pub struct TorrentData<I: Ip> {
    peers: PeerMap<I>,
    num_seeders: usize,
}

impl<I: Ip> TorrentData<I> {
    pub fn announce(
        &mut self,
        config: &Config,
        statistics_sender: &Sender<StatisticsMessage>,
        rng: &mut SmallRng,
        request: &AnnounceRequest,
        ip_address: I,
        valid_until: ValidUntil,
        response: &mut AnnounceResponse<I>,
    ) {
        let max_num_peers_to_take: usize = if request.peers_wanted.0.get() <= 0 {
            config.protocol.max_response_peers
        } else {
            ::std::cmp::min(
                config.protocol.max_response_peers,
                request.peers_wanted.0.get().try_into().unwrap(),
            )
        };

        let status =
            PeerStatus::from_event_and_bytes_left(request.event.into(), request.bytes_left);

        let opt_removed_peer = self.peers.remove(&request.peer_id);

        if let Some(Peer {
            is_seeder: true, ..
        }) = opt_removed_peer
        {
            self.num_seeders -= 1;
        }

        // Create the response before inserting the peer. This means that we
        // don't have to filter it out from the response peers, and that the
        // reported number of seeders/leechers will not include it

        response.fixed = AnnounceResponseFixedData {
            transaction_id: request.transaction_id,
            announce_interval: AnnounceInterval::new(config.protocol.peer_announce_interval),
            leechers: NumberOfPeers::new(self.num_leechers().try_into().unwrap_or(i32::MAX)),
            seeders: NumberOfPeers::new(self.num_seeders().try_into().unwrap_or(i32::MAX)),
        };

        extract_response_peers(
            rng,
            &self.peers,
            max_num_peers_to_take,
            Peer::to_response_peer,
            &mut response.peers,
        );

        match status {
            PeerStatus::Leeching => {
                let peer = Peer {
                    ip_address,
                    port: request.port,
                    is_seeder: false,
                    valid_until,
                };

                self.peers.insert(request.peer_id, peer);

                if config.statistics.peer_clients && opt_removed_peer.is_none() {
                    statistics_sender
                        .try_send(StatisticsMessage::PeerAdded(request.peer_id))
                        .expect("statistics channel should be unbounded");
                }
            }
            PeerStatus::Seeding => {
                let peer = Peer {
                    ip_address,
                    port: request.port,
                    is_seeder: true,
                    valid_until,
                };

                self.peers.insert(request.peer_id, peer);

                self.num_seeders += 1;

                if config.statistics.peer_clients && opt_removed_peer.is_none() {
                    statistics_sender
                        .try_send(StatisticsMessage::PeerAdded(request.peer_id))
                        .expect("statistics channel should be unbounded");
                }
            }
            PeerStatus::Stopped => {
                if config.statistics.peer_clients && opt_removed_peer.is_some() {
                    statistics_sender
                        .try_send(StatisticsMessage::PeerRemoved(request.peer_id))
                        .expect("statistics channel should be unbounded");
                }
            }
        };
    }

    pub fn num_leechers(&self) -> usize {
        self.peers.len() - self.num_seeders
    }

    pub fn num_seeders(&self) -> usize {
        self.num_seeders
    }

    pub fn scrape_statistics(&self) -> TorrentScrapeStatistics {
        create_torrent_scrape_statistics(
            self.num_seeders.try_into().unwrap_or(i32::MAX),
            self.num_leechers().try_into().unwrap_or(i32::MAX),
        )
    }

    /// Remove inactive peers and reclaim space
    fn clean(
        &mut self,
        config: &Config,
        statistics_sender: &Sender<StatisticsMessage>,
        now: SecondsSinceServerStart,
    ) {
        self.peers.retain(|peer_id, peer| {
            let keep = peer.valid_until.valid(now);

            if !keep {
                if peer.is_seeder {
                    self.num_seeders -= 1;
                }
                if config.statistics.peer_clients {
                    if let Err(_) =
                        statistics_sender.try_send(StatisticsMessage::PeerRemoved(*peer_id))
                    {
                        // Should never happen in practice
                        ::log::error!("Couldn't send StatisticsMessage::PeerRemoved");
                    }
                }
            }

            keep
        });

        if !self.peers.is_empty() {
            self.peers.shrink_to_fit();
        }
    }
}

impl<I: Ip> Default for TorrentData<I> {
    fn default() -> Self {
        Self {
            peers: Default::default(),
            num_seeders: 0,
        }
    }
}

#[derive(Default)]
pub struct TorrentMap<I: Ip>(pub IndexMap<InfoHash, TorrentData<I>>);

impl<I: Ip> TorrentMap<I> {
    pub fn scrape(&mut self, request: PendingScrapeRequest, response: &mut PendingScrapeResponse) {
        response.slab_key = request.slab_key;

        let torrent_stats = request.info_hashes.into_iter().map(|(i, info_hash)| {
            let stats = self
                .0
                .get(&info_hash)
                .map(|torrent_data| torrent_data.scrape_statistics())
                .unwrap_or_else(|| create_torrent_scrape_statistics(0, 0));

            (i, stats)
        });

        response.torrent_stats.extend(torrent_stats);
    }
    /// Remove forbidden or inactive torrents, reclaim space and return number of remaining peers
    fn clean_and_get_statistics(
        &mut self,
        config: &Config,
        statistics_sender: &Sender<StatisticsMessage>,
        access_list_cache: &mut AccessListCache,
        access_list_mode: AccessListMode,
        now: SecondsSinceServerStart,
    ) -> (usize, Option<Histogram<u64>>) {
        let mut num_peers = 0;

        let mut opt_histogram: Option<Histogram<u64>> = if config.statistics.torrent_peer_histograms
        {
            match Histogram::new(3) {
                Ok(histogram) => Some(histogram),
                Err(err) => {
                    ::log::error!("Couldn't create peer histogram: {:#}", err);

                    None
                }
            }
        } else {
            None
        };

        self.0.retain(|info_hash, torrent| {
            if !access_list_cache
                .load()
                .allows(access_list_mode, &info_hash.0)
            {
                return false;
            }

            torrent.clean(config, statistics_sender, now);

            num_peers += torrent.peers.len();

            match opt_histogram {
                Some(ref mut histogram) if torrent.peers.len() != 0 => {
                    let n = torrent
                        .peers
                        .len()
                        .try_into()
                        .expect("Couldn't fit usize into u64");

                    if let Err(err) = histogram.record(n) {
                        ::log::error!("Couldn't record {} to histogram: {:#}", n, err);
                    }
                }
                _ => (),
            }

            !torrent.peers.is_empty()
        });

        self.0.shrink_to_fit();

        (num_peers, opt_histogram)
    }

    pub fn num_torrents(&self) -> usize {
        self.0.len()
    }
}

pub struct TorrentMaps {
    pub ipv4: TorrentMap<Ipv4AddrBytes>,
    pub ipv6: TorrentMap<Ipv6AddrBytes>,
}

impl Default for TorrentMaps {
    fn default() -> Self {
        Self {
            ipv4: TorrentMap(Default::default()),
            ipv6: TorrentMap(Default::default()),
        }
    }
}

impl TorrentMaps {
    /// Remove forbidden or inactive torrents, reclaim space and update statistics
    pub fn clean_and_update_statistics(
        &mut self,
        config: &Config,
        state: &State,
        statistics_sender: &Sender<StatisticsMessage>,
        access_list: &Arc<AccessListArcSwap>,
        server_start_instant: ServerStartInstant,
        worker_index: SwarmWorkerIndex,
    ) {
        let mut cache = create_access_list_cache(access_list);
        let mode = config.access_list.mode;
        let now = server_start_instant.seconds_elapsed();

        let ipv4 =
            self.ipv4
                .clean_and_get_statistics(config, statistics_sender, &mut cache, mode, now);
        let ipv6 =
            self.ipv6
                .clean_and_get_statistics(config, statistics_sender, &mut cache, mode, now);

        if config.statistics.active() {
            state.statistics_ipv4.peers[worker_index.0].store(ipv4.0, Ordering::Release);
            state.statistics_ipv6.peers[worker_index.0].store(ipv6.0, Ordering::Release);

            if let Some(message) = ipv4.1.map(StatisticsMessage::Ipv4PeerHistogram) {
                if let Err(err) = statistics_sender.try_send(message) {
                    ::log::error!("couldn't send statistics message: {:#}", err);
                }
            }
            if let Some(message) = ipv6.1.map(StatisticsMessage::Ipv6PeerHistogram) {
                if let Err(err) = statistics_sender.try_send(message) {
                    ::log::error!("couldn't send statistics message: {:#}", err);
                }
            }
        }
    }
}
/// Extract response peers
///
/// If there are more peers in map than `max_num_peers_to_take`, do a random
/// selection of peers from first and second halves of map in order to avoid
/// returning too homogeneous peers.
///
/// Does NOT filter out announcing peer.
#[inline]
pub fn extract_response_peers<K, V, R, F>(
    rng: &mut impl Rng,
    peer_map: &IndexMap<K, V>,
    max_num_peers_to_take: usize,
    peer_conversion_function: F,
    peers: &mut Vec<R>,
) where
    K: Eq + ::std::hash::Hash,
    F: Fn(&K, &V) -> R,
{
    if peer_map.len() <= max_num_peers_to_take {
        peers.extend(peer_map.iter().map(|(k, v)| peer_conversion_function(k, v)));
    } else {
        let middle_index = peer_map.len() / 2;
        let num_to_take_per_half = max_num_peers_to_take / 2;

        let offset_half_one = {
            let from = 0;
            let to = usize::max(1, middle_index - num_to_take_per_half);

            rng.gen_range(from..to)
        };
        let offset_half_two = {
            let from = middle_index;
            let to = usize::max(middle_index + 1, peer_map.len() - num_to_take_per_half);

            rng.gen_range(from..to)
        };

        let end_half_one = offset_half_one + num_to_take_per_half;
        let end_half_two = offset_half_two + num_to_take_per_half;

        if let Some(slice) = peer_map.get_range(offset_half_one..end_half_one) {
            peers.extend(slice.iter().map(|(k, v)| peer_conversion_function(k, v)));
        }
        if let Some(slice) = peer_map.get_range(offset_half_two..end_half_two) {
            peers.extend(slice.iter().map(|(k, v)| peer_conversion_function(k, v)));
        }
    }
}

#[inline(always)]
fn create_torrent_scrape_statistics(seeders: i32, leechers: i32) -> TorrentScrapeStatistics {
    TorrentScrapeStatistics {
        seeders: NumberOfPeers::new(seeders),
        completed: NumberOfDownloads::new(0), // No implementation planned
        leechers: NumberOfPeers::new(leechers),
    }
}
