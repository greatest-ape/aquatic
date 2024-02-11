use std::iter::repeat_with;
use std::net::IpAddr;
use std::ops::DerefMut;
use std::sync::atomic::AtomicBool;
use std::sync::atomic::Ordering;
use std::sync::Arc;

use aquatic_common::SecondsSinceServerStart;
use aquatic_common::ServerStartInstant;
use aquatic_common::{
    access_list::{create_access_list_cache, AccessListArcSwap, AccessListCache, AccessListMode},
    ValidUntil,
};
use aquatic_common::{CanonicalSocketAddr, IndexMap};

use aquatic_udp_protocol::*;
use arrayvec::ArrayVec;
use crossbeam_channel::Sender;
use hashbrown::HashMap;
use hdrhistogram::Histogram;
use parking_lot::RwLockUpgradableReadGuard;
use rand::prelude::SmallRng;
use rand::Rng;

use crate::common::*;
use crate::config::Config;

const SMALL_PEER_MAP_CAPACITY: usize = 2;

use aquatic_udp_protocol::InfoHash;
use parking_lot::RwLock;

#[derive(Clone)]
pub struct TorrentMaps {
    ipv4: TorrentMapShards<Ipv4AddrBytes>,
    ipv6: TorrentMapShards<Ipv6AddrBytes>,
}

impl Default for TorrentMaps {
    fn default() -> Self {
        const NUM_SHARDS: usize = 16;

        Self {
            ipv4: TorrentMapShards::new(NUM_SHARDS),
            ipv6: TorrentMapShards::new(NUM_SHARDS),
        }
    }
}

impl TorrentMaps {
    pub fn announce(
        &self,
        config: &Config,
        statistics_sender: &Sender<StatisticsMessage>,
        rng: &mut SmallRng,
        request: &AnnounceRequest,
        src: CanonicalSocketAddr,
        valid_until: ValidUntil,
    ) -> Response {
        match src.get().ip() {
            IpAddr::V4(ip_address) => Response::AnnounceIpv4(self.ipv4.announce(
                config,
                statistics_sender,
                rng,
                request,
                ip_address.into(),
                valid_until,
            )),
            IpAddr::V6(ip_address) => Response::AnnounceIpv6(self.ipv6.announce(
                config,
                statistics_sender,
                rng,
                request,
                ip_address.into(),
                valid_until,
            )),
        }
    }

    pub fn scrape(&self, request: ScrapeRequest, src: CanonicalSocketAddr) -> ScrapeResponse {
        if src.is_ipv4() {
            self.ipv4.scrape(request)
        } else {
            self.ipv6.scrape(request)
        }
    }

    /// Remove forbidden or inactive torrents, reclaim space and update statistics
    pub fn clean_and_update_statistics(
        &self,
        config: &Config,
        statistics: &CachePaddedArc<IpVersionStatistics<SwarmWorkerStatistics>>,
        statistics_sender: &Sender<StatisticsMessage>,
        access_list: &Arc<AccessListArcSwap>,
        server_start_instant: ServerStartInstant,
    ) {
        let mut cache = create_access_list_cache(access_list);
        let mode = config.access_list.mode;
        let now = server_start_instant.seconds_elapsed();

        let mut statistics_messages = Vec::new();

        let ipv4 = self.ipv4.clean_and_get_statistics(
            config,
            &mut statistics_messages,
            &mut cache,
            mode,
            now,
        );
        let ipv6 = self.ipv6.clean_and_get_statistics(
            config,
            &mut statistics_messages,
            &mut cache,
            mode,
            now,
        );

        if config.statistics.active() {
            statistics.ipv4.torrents.store(ipv4.0, Ordering::Relaxed);
            statistics.ipv6.torrents.store(ipv6.0, Ordering::Relaxed);
            statistics.ipv4.peers.store(ipv4.1, Ordering::Relaxed);
            statistics.ipv6.peers.store(ipv6.1, Ordering::Relaxed);

            if let Some(message) = ipv4.2 {
                statistics_messages.push(StatisticsMessage::Ipv4PeerHistogram(message));
            }
            if let Some(message) = ipv6.2 {
                statistics_messages.push(StatisticsMessage::Ipv6PeerHistogram(message));
            }

            for message in statistics_messages {
                if let Err(err) = statistics_sender.try_send(message) {
                    ::log::error!("couldn't send statistics message: {:#}", err);
                }
            }
        }
    }
}

#[derive(Clone)]
pub struct TorrentMapShards<I: Ip>(Arc<[RwLock<TorrentMapShard<I>>]>);

impl<I: Ip> TorrentMapShards<I> {
    fn new(num_shards: usize) -> Self {
        Self(
            repeat_with(Default::default)
                .take(num_shards)
                .collect::<Vec<_>>()
                .into_boxed_slice()
                .into(),
        )
    }

    fn announce(
        &self,
        config: &Config,
        statistics_sender: &Sender<StatisticsMessage>,
        rng: &mut SmallRng,
        request: &AnnounceRequest,
        ip_address: I,
        valid_until: ValidUntil,
    ) -> AnnounceResponse<I> {
        let torrent_data = {
            let torrent_map_shard = self.get_shard(&request.info_hash).upgradable_read();

            // Clone Arc here to avoid keeping lock on whole shard
            if let Some(torrent_data) = torrent_map_shard.get(&request.info_hash) {
                torrent_data.clone()
            } else {
                // Don't overwrite entry if created in the meantime
                RwLockUpgradableReadGuard::upgrade(torrent_map_shard)
                    .entry(request.info_hash)
                    .or_default()
                    .clone()
            }
        };

        let mut peer_map = torrent_data.peer_map.write();

        peer_map.announce(
            config,
            statistics_sender,
            rng,
            request,
            ip_address,
            valid_until,
        )
    }

    fn scrape(&self, request: ScrapeRequest) -> ScrapeResponse {
        let mut response = ScrapeResponse {
            transaction_id: request.transaction_id,
            torrent_stats: Vec::with_capacity(request.info_hashes.len()),
        };

        for info_hash in request.info_hashes {
            let torrent_map_shard = self.get_shard(&info_hash);

            let statistics = if let Some(torrent_data) = torrent_map_shard.read().get(&info_hash) {
                torrent_data.peer_map.read().scrape_statistics()
            } else {
                TorrentScrapeStatistics {
                    seeders: NumberOfPeers::new(0),
                    leechers: NumberOfPeers::new(0),
                    completed: NumberOfDownloads::new(0),
                }
            };

            response.torrent_stats.push(statistics);
        }

        response
    }

    fn clean_and_get_statistics(
        &self,
        config: &Config,
        statistics_messages: &mut Vec<StatisticsMessage>,
        access_list_cache: &mut AccessListCache,
        access_list_mode: AccessListMode,
        now: SecondsSinceServerStart,
    ) -> (usize, usize, Option<Histogram<u64>>) {
        let mut total_num_torrents = 0;
        let mut total_num_peers = 0;

        let mut opt_histogram: Option<Histogram<u64>> = config
            .statistics
            .torrent_peer_histograms
            .then(|| Histogram::new(3).expect("create peer histogram"));

        for torrent_map_shard in self.0.iter() {
            for torrent_data in torrent_map_shard.read().values() {
                let mut peer_map = torrent_data.peer_map.write();

                let num_peers = match peer_map.deref_mut() {
                    PeerMap::Small(small_peer_map) => {
                        small_peer_map.clean_and_get_num_peers(config, statistics_messages, now)
                    }
                    PeerMap::Large(large_peer_map) => {
                        let num_peers = large_peer_map.clean_and_get_num_peers(
                            config,
                            statistics_messages,
                            now,
                        );

                        if let Some(small_peer_map) = large_peer_map.try_shrink() {
                            *peer_map = PeerMap::Small(small_peer_map);
                        }

                        num_peers
                    }
                };

                drop(peer_map);

                match opt_histogram.as_mut() {
                    Some(histogram) if num_peers > 0 => {
                        if let Err(err) = histogram.record(num_peers as u64) {
                            ::log::error!("Couldn't record {} to histogram: {:#}", num_peers, err);
                        }
                    }
                    _ => (),
                }

                total_num_peers += num_peers;

                torrent_data
                    .pending_removal
                    .store(num_peers == 0, Ordering::Release);
            }

            let mut torrent_map_shard = torrent_map_shard.write();

            torrent_map_shard.retain(|info_hash, torrent_data| {
                if !access_list_cache
                    .load()
                    .allows(access_list_mode, &info_hash.0)
                {
                    return false;
                }

                // Check pending_removal flag set in previous cleaning step. This
                // prevents us from removing TorrentData entries that were just
                // added but do not yet contain any peers. Also double-check that
                // no peers have been added since we last checked.
                if torrent_data
                    .pending_removal
                    .fetch_and(false, Ordering::Acquire)
                    && torrent_data.peer_map.read().is_empty()
                {
                    return false;
                }

                true
            });

            torrent_map_shard.shrink_to_fit();

            total_num_torrents += torrent_map_shard.len();
        }

        (total_num_torrents, total_num_peers, opt_histogram)
    }

    fn get_shard(&self, info_hash: &InfoHash) -> &RwLock<TorrentMapShard<I>> {
        self.0.get(info_hash.0[0] as usize % self.0.len()).unwrap()
    }
}

/// Use HashMap instead of IndexMap for better lookup performance
type TorrentMapShard<T> = HashMap<InfoHash, Arc<TorrentData<T>>>;

pub struct TorrentData<T: Ip> {
    peer_map: RwLock<PeerMap<T>>,
    pending_removal: AtomicBool,
}

impl<I: Ip> Default for TorrentData<I> {
    fn default() -> Self {
        Self {
            peer_map: Default::default(),
            pending_removal: Default::default(),
        }
    }
}

pub enum PeerMap<I: Ip> {
    Small(SmallPeerMap<I>),
    Large(LargePeerMap<I>),
}

impl<I: Ip> PeerMap<I> {
    fn announce(
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

    fn scrape_statistics(&self) -> TorrentScrapeStatistics {
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

    fn is_empty(&self) -> bool {
        match self {
            Self::Small(peer_map) => peer_map.0.is_empty(),
            Self::Large(peer_map) => peer_map.peers.is_empty(),
        }
    }
}

impl<I: Ip> Default for PeerMap<I> {
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
        statistics_messages: &mut Vec<StatisticsMessage>,
        now: SecondsSinceServerStart,
    ) -> usize {
        self.0.retain(|(_, peer)| {
            let keep = peer.valid_until.valid(now);

            if !keep && config.statistics.peer_clients {
                statistics_messages.push(StatisticsMessage::PeerRemoved(peer.peer_id));
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
        let opt_removed_peer = self.peers.swap_remove(key);

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
    /// If there are more peers in map than `max_num_peers_to_take`, do a
    /// random selection of peers from first and second halves of map in
    /// order to avoid returning too homogeneous peers. This is a lot more
    /// cache-friendly than doing a fully random selection.
    fn extract_response_peers(
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
                peers.extend(slice.keys().copied());
            }
            if let Some(slice) = self.peers.get_range(offset_half_two..end_half_two) {
                peers.extend(slice.keys().copied());
            }

            peers
        }
    }

    fn clean_and_get_num_peers(
        &mut self,
        config: &Config,
        statistics_messages: &mut Vec<StatisticsMessage>,
        now: SecondsSinceServerStart,
    ) -> usize {
        self.peers.retain(|_, peer| {
            let keep = peer.valid_until.valid(now);

            if !keep {
                if peer.is_seeder {
                    self.num_seeders -= 1;
                }
                if config.statistics.peer_clients {
                    statistics_messages.push(StatisticsMessage::PeerRemoved(peer.peer_id));
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

#[derive(PartialEq, Eq, Hash, Clone, Copy, Debug)]
pub enum PeerStatus {
    Seeding,
    Leeching,
    Stopped,
}

impl PeerStatus {
    /// Determine peer status from announce event and number of bytes left.
    ///
    /// Likely, the last branch will be taken most of the time.
    #[inline]
    pub fn from_event_and_bytes_left(event: AnnounceEvent, bytes_left: NumberOfBytes) -> Self {
        if event == AnnounceEvent::Stopped {
            Self::Stopped
        } else if bytes_left.0.get() == 0 {
            Self::Seeding
        } else {
            Self::Leeching
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_peer_status_from_event_and_bytes_left() {
        use PeerStatus::*;

        let f = PeerStatus::from_event_and_bytes_left;

        assert_eq!(Stopped, f(AnnounceEvent::Stopped, NumberOfBytes::new(0)));
        assert_eq!(Stopped, f(AnnounceEvent::Stopped, NumberOfBytes::new(1)));

        assert_eq!(Seeding, f(AnnounceEvent::Started, NumberOfBytes::new(0)));
        assert_eq!(Leeching, f(AnnounceEvent::Started, NumberOfBytes::new(1)));

        assert_eq!(Seeding, f(AnnounceEvent::Completed, NumberOfBytes::new(0)));
        assert_eq!(Leeching, f(AnnounceEvent::Completed, NumberOfBytes::new(1)));

        assert_eq!(Seeding, f(AnnounceEvent::None, NumberOfBytes::new(0)));
        assert_eq!(Leeching, f(AnnounceEvent::None, NumberOfBytes::new(1)));
    }
}
