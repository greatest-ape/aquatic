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
use arrayvec::ArrayVec;
use crossbeam_channel::Sender;
use hdrhistogram::Histogram;
use rand::prelude::SmallRng;
use rand::Rng;

use crate::common::*;
use crate::config::Config;

const SMALL_PEER_MAP_CAPACITY: usize = 2;

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

#[derive(Default)]
pub struct TorrentMap<I: Ip>(pub IndexMap<InfoHash, TorrentData<I>>);

impl<I: Ip> TorrentMap<I> {
    pub fn scrape(&mut self, request: PendingScrapeRequest) -> PendingScrapeResponse {
        let torrent_stats = request
            .info_hashes
            .into_iter()
            .map(|(i, info_hash)| {
                let stats = self
                    .0
                    .get(&info_hash)
                    .map(|torrent_data| torrent_data.scrape_statistics())
                    .unwrap_or_else(|| TorrentScrapeStatistics {
                        seeders: NumberOfPeers::new(0),
                        leechers: NumberOfPeers::new(0),
                        completed: NumberOfDownloads::new(0),
                    });

                (i, stats)
            })
            .collect();

        PendingScrapeResponse {
            slab_key: request.slab_key,
            torrent_stats,
        }
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
        let mut total_num_peers = 0;

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

            let num_peers = match torrent {
                TorrentData::Small(peer_map) => {
                    peer_map.clean_and_get_num_peers(config, statistics_sender, now)
                }
                TorrentData::Large(peer_map) => {
                    let num_peers =
                        peer_map.clean_and_get_num_peers(config, statistics_sender, now);

                    if let Some(peer_map) = peer_map.try_shrink() {
                        *torrent = TorrentData::Small(peer_map);
                    }

                    num_peers
                }
            };

            total_num_peers += num_peers;

            match opt_histogram {
                Some(ref mut histogram) if num_peers > 0 => {
                    let n = num_peers.try_into().expect("Couldn't fit usize into u64");

                    if let Err(err) = histogram.record(n) {
                        ::log::error!("Couldn't record {} to histogram: {:#}", n, err);
                    }
                }
                _ => (),
            }

            num_peers > 0
        });

        self.0.shrink_to_fit();

        (total_num_peers, opt_histogram)
    }

    pub fn num_torrents(&self) -> usize {
        self.0.len()
    }
}

pub enum TorrentData<I: Ip> {
    Small(SmallPeerMap<I>),
    Large(LargePeerMap<I>),
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
    ) -> AnnounceResponse<I> {
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

        let peer_map_key = ResponsePeer {
            ip_address,
            port: request.port,
        };

        // Create the response before inserting the peer. This means that we
        // don't have to filter it out from the response peers, and that the
        // reported number of seeders/leechers will not include it
        let (response, opt_removed_peer) = match self {
            Self::Small(peer_map) => {
                let opt_removed_peer = peer_map.remove(&peer_map_key);

                let (seeders, leechers) = peer_map.num_seeders_leechers();

                let response = AnnounceResponse {
                    fixed: AnnounceResponseFixedData {
                        transaction_id: request.transaction_id,
                        announce_interval: AnnounceInterval::new(
                            config.protocol.peer_announce_interval,
                        ),
                        leechers: NumberOfPeers::new(leechers.try_into().unwrap_or(i32::MAX)),
                        seeders: NumberOfPeers::new(seeders.try_into().unwrap_or(i32::MAX)),
                    },
                    peers: peer_map.extract_response_peers(max_num_peers_to_take),
                };

                // Convert peer map to large variant if it is full and
                // announcing peer is not stopped and will therefore be
                // inserted
                if peer_map.is_full() && status != PeerStatus::Stopped {
                    *self = Self::Large(peer_map.to_large());
                }

                (response, opt_removed_peer)
            }
            Self::Large(peer_map) => {
                let opt_removed_peer = peer_map.remove_peer(&peer_map_key);

                let (seeders, leechers) = peer_map.num_seeders_leechers();

                let response = AnnounceResponse {
                    fixed: AnnounceResponseFixedData {
                        transaction_id: request.transaction_id,
                        announce_interval: AnnounceInterval::new(
                            config.protocol.peer_announce_interval,
                        ),
                        leechers: NumberOfPeers::new(leechers.try_into().unwrap_or(i32::MAX)),
                        seeders: NumberOfPeers::new(seeders.try_into().unwrap_or(i32::MAX)),
                    },
                    peers: peer_map.extract_response_peers(rng, max_num_peers_to_take),
                };

                // Try shrinking the map if announcing peer is stopped and
                // will therefore not be inserted
                if status == PeerStatus::Stopped {
                    if let Some(peer_map) = peer_map.try_shrink() {
                        *self = Self::Small(peer_map);
                    }
                }

                (response, opt_removed_peer)
            }
        };

        match status {
            PeerStatus::Leeching | PeerStatus::Seeding => {
                let peer = Peer {
                    peer_id: request.peer_id,
                    is_seeder: status == PeerStatus::Seeding,
                    valid_until,
                };

                match self {
                    Self::Small(peer_map) => peer_map.insert(peer_map_key, peer),
                    Self::Large(peer_map) => peer_map.insert(peer_map_key, peer),
                }

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

        response
    }

    pub fn scrape_statistics(&self) -> TorrentScrapeStatistics {
        let (seeders, leechers) = match self {
            Self::Small(peer_map) => peer_map.num_seeders_leechers(),
            Self::Large(peer_map) => peer_map.num_seeders_leechers(),
        };

        TorrentScrapeStatistics {
            seeders: NumberOfPeers::new(seeders.try_into().unwrap_or(i32::MAX)),
            leechers: NumberOfPeers::new(leechers.try_into().unwrap_or(i32::MAX)),
            completed: NumberOfDownloads::new(0),
        }
    }
}

impl<I: Ip> Default for TorrentData<I> {
    fn default() -> Self {
        Self::Small(SmallPeerMap(ArrayVec::default()))
    }
}

/// Store torrents with up to two peers without an extra heap allocation
///
/// On public open trackers, this is likely to be the majority of torrents.
#[derive(Default, Debug)]
pub struct SmallPeerMap<I: Ip>(ArrayVec<(ResponsePeer<I>, Peer), SMALL_PEER_MAP_CAPACITY>);

impl<I: Ip> SmallPeerMap<I> {
    fn is_full(&self) -> bool {
        self.0.is_full()
    }

    fn num_seeders_leechers(&self) -> (usize, usize) {
        let seeders = self.0.iter().filter(|(_, p)| p.is_seeder).count();
        let leechers = self.0.len() - seeders;

        (seeders, leechers)
    }

    fn insert(&mut self, key: ResponsePeer<I>, peer: Peer) {
        self.0.push((key, peer));
    }

    fn remove(&mut self, key: &ResponsePeer<I>) -> Option<Peer> {
        for (i, (k, _)) in self.0.iter().enumerate() {
            if k == key {
                return Some(self.0.remove(i).1);
            }
        }

        None
    }

    fn extract_response_peers(&self, max_num_peers_to_take: usize) -> Vec<ResponsePeer<I>> {
        Vec::from_iter(self.0.iter().take(max_num_peers_to_take).map(|(k, _)| *k))
    }

    fn clean_and_get_num_peers(
        &mut self,
        config: &Config,
        statistics_sender: &Sender<StatisticsMessage>,
        now: SecondsSinceServerStart,
    ) -> usize {
        self.0.retain(|(_, peer)| {
            let keep = peer.valid_until.valid(now);

            if !keep && config.statistics.peer_clients {
                if let Err(_) =
                    statistics_sender.try_send(StatisticsMessage::PeerRemoved(peer.peer_id))
                {
                    // Should never happen in practice
                    ::log::error!("Couldn't send StatisticsMessage::PeerRemoved");
                }
            }

            keep
        });

        self.0.len()
    }

    fn to_large(&self) -> LargePeerMap<I> {
        let (num_seeders, _) = self.num_seeders_leechers();
        let peers = self.0.iter().copied().collect();

        LargePeerMap { peers, num_seeders }
    }
}

#[derive(Default)]
pub struct LargePeerMap<I: Ip> {
    peers: IndexMap<ResponsePeer<I>, Peer>,
    num_seeders: usize,
}

impl<I: Ip> LargePeerMap<I> {
    fn num_seeders_leechers(&self) -> (usize, usize) {
        (self.num_seeders, self.peers.len() - self.num_seeders)
    }

    fn insert(&mut self, key: ResponsePeer<I>, peer: Peer) {
        if peer.is_seeder {
            self.num_seeders += 1;
        }

        self.peers.insert(key, peer);
    }

    fn remove_peer(&mut self, key: &ResponsePeer<I>) -> Option<Peer> {
        let opt_removed_peer = self.peers.remove(key);

        if let Some(Peer {
            is_seeder: true, ..
        }) = opt_removed_peer
        {
            self.num_seeders -= 1;
        }

        opt_removed_peer
    }

    /// Extract response peers
    ///
    /// If there are more peers in map than `max_num_peers_to_take`, do a random
    /// selection of peers from first and second halves of map in order to avoid
    /// returning too homogeneous peers.
    ///
    /// Does NOT filter out announcing peer.
    pub fn extract_response_peers(
        &self,
        rng: &mut impl Rng,
        max_num_peers_to_take: usize,
    ) -> Vec<ResponsePeer<I>> {
        if self.peers.len() <= max_num_peers_to_take {
            self.peers.keys().copied().collect()
        } else {
            let middle_index = self.peers.len() / 2;
            let num_to_take_per_half = max_num_peers_to_take / 2;

            let offset_half_one = {
                let from = 0;
                let to = usize::max(1, middle_index - num_to_take_per_half);

                rng.gen_range(from..to)
            };
            let offset_half_two = {
                let from = middle_index;
                let to = usize::max(middle_index + 1, self.peers.len() - num_to_take_per_half);

                rng.gen_range(from..to)
            };

            let end_half_one = offset_half_one + num_to_take_per_half;
            let end_half_two = offset_half_two + num_to_take_per_half;

            let mut peers = Vec::with_capacity(max_num_peers_to_take);

            if let Some(slice) = self.peers.get_range(offset_half_one..end_half_one) {
                peers.extend(slice.keys());
            }
            if let Some(slice) = self.peers.get_range(offset_half_two..end_half_two) {
                peers.extend(slice.keys());
            }

            peers
        }
    }

    fn clean_and_get_num_peers(
        &mut self,
        config: &Config,
        statistics_sender: &Sender<StatisticsMessage>,
        now: SecondsSinceServerStart,
    ) -> usize {
        self.peers.retain(|_, peer| {
            let keep = peer.valid_until.valid(now);

            if !keep {
                if peer.is_seeder {
                    self.num_seeders -= 1;
                }
                if config.statistics.peer_clients {
                    if let Err(_) =
                        statistics_sender.try_send(StatisticsMessage::PeerRemoved(peer.peer_id))
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

        self.peers.len()
    }

    fn try_shrink(&mut self) -> Option<SmallPeerMap<I>> {
        (self.peers.len() <= SMALL_PEER_MAP_CAPACITY).then(|| {
            SmallPeerMap(ArrayVec::from_iter(
                self.peers.iter().map(|(k, v)| (*k, *v)),
            ))
        })
    }
}

#[derive(Clone, Copy, Debug)]
struct Peer {
    peer_id: PeerId,
    is_seeder: bool,
    valid_until: ValidUntil,
}
