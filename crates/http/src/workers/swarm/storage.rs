use std::collections::BTreeMap;
use std::net::{IpAddr, Ipv4Addr, Ipv6Addr};
use std::sync::Arc;

use arrayvec::ArrayVec;
use rand::Rng;

use aquatic_common::access_list::{create_access_list_cache, AccessListArcSwap, AccessListCache};
use aquatic_common::{
    CanonicalSocketAddr, IndexMap, SecondsSinceServerStart, ServerStartInstant, ValidUntil,
};
use aquatic_http_protocol::common::*;
use aquatic_http_protocol::request::*;
use aquatic_http_protocol::response::ResponsePeer;
use aquatic_http_protocol::response::*;

use crate::config::Config;

const SMALL_PEER_MAP_CAPACITY: usize = 4;

pub trait Ip: ::std::fmt::Debug + Copy + Eq + ::std::hash::Hash {}

impl Ip for Ipv4Addr {}
impl Ip for Ipv6Addr {}

pub struct TorrentMaps {
    pub ipv4: TorrentMap<Ipv4Addr>,
    pub ipv6: TorrentMap<Ipv6Addr>,
}

impl TorrentMaps {
    pub fn new(worker_index: usize) -> Self {
        Self {
            ipv4: TorrentMap::new(worker_index, true),
            ipv6: TorrentMap::new(worker_index, false),
        }
    }

    pub fn handle_announce_request(
        &mut self,
        config: &Config,
        rng: &mut impl Rng,
        valid_until: ValidUntil,
        peer_addr: CanonicalSocketAddr,
        request: AnnounceRequest,
    ) -> AnnounceResponse {
        match peer_addr.get().ip() {
            IpAddr::V4(peer_ip_address) => {
                let (seeders, leechers, response_peers) =
                    self.ipv4.upsert_peer_and_get_response_peers(
                        config,
                        rng,
                        valid_until,
                        peer_ip_address,
                        request,
                    );

                AnnounceResponse {
                    complete: seeders,
                    incomplete: leechers,
                    announce_interval: config.protocol.peer_announce_interval,
                    peers: ResponsePeerListV4(response_peers),
                    peers6: ResponsePeerListV6(vec![]),
                    warning_message: None,
                }
            }
            IpAddr::V6(peer_ip_address) => {
                let (seeders, leechers, response_peers) =
                    self.ipv6.upsert_peer_and_get_response_peers(
                        config,
                        rng,
                        valid_until,
                        peer_ip_address,
                        request,
                    );

                AnnounceResponse {
                    complete: seeders,
                    incomplete: leechers,
                    announce_interval: config.protocol.peer_announce_interval,
                    peers: ResponsePeerListV4(vec![]),
                    peers6: ResponsePeerListV6(response_peers),
                    warning_message: None,
                }
            }
        }
    }

    pub fn handle_scrape_request(
        &mut self,
        config: &Config,
        peer_addr: CanonicalSocketAddr,
        request: ScrapeRequest,
    ) -> ScrapeResponse {
        if peer_addr.get().ip().is_ipv4() {
            self.ipv4.handle_scrape_request(config, request)
        } else {
            self.ipv6.handle_scrape_request(config, request)
        }
    }

    #[cfg(feature = "metrics")]
    pub fn update_torrent_metrics(&self) {
        self.ipv4.torrent_gauge.set(self.ipv4.torrents.len() as f64);
        self.ipv6.torrent_gauge.set(self.ipv6.torrents.len() as f64);
    }

    pub fn clean(
        &mut self,
        config: &Config,
        access_list: &Arc<AccessListArcSwap>,
        server_start_instant: ServerStartInstant,
    ) {
        let mut access_list_cache = create_access_list_cache(access_list);

        let now = server_start_instant.seconds_elapsed();

        self.ipv4.clean(config, &mut access_list_cache, now);
        self.ipv6.clean(config, &mut access_list_cache, now);
    }
}

pub struct TorrentMap<I: Ip> {
    torrents: IndexMap<InfoHash, TorrentData<I>>,
    #[cfg(feature = "metrics")]
    peer_gauge: ::metrics::Gauge,
    #[cfg(feature = "metrics")]
    torrent_gauge: ::metrics::Gauge,
}

impl<I: Ip> TorrentMap<I> {
    fn new(worker_index: usize, ipv4: bool) -> Self {
        #[cfg(feature = "metrics")]
        let peer_gauge = if ipv4 {
            ::metrics::gauge!(
                "aquatic_peers",
                "ip_version" => "4",
                "worker_index" => worker_index.to_string(),
            )
        } else {
            ::metrics::gauge!(
                "aquatic_peers",
                "ip_version" => "6",
                "worker_index" => worker_index.to_string(),
            )
        };
        #[cfg(feature = "metrics")]
        let torrent_gauge = if ipv4 {
            ::metrics::gauge!(
                "aquatic_torrents",
                "ip_version" => "4",
                "worker_index" => worker_index.to_string(),
            )
        } else {
            ::metrics::gauge!(
                "aquatic_torrents",
                "ip_version" => "6",
                "worker_index" => worker_index.to_string(),
            )
        };

        Self {
            torrents: Default::default(),
            #[cfg(feature = "metrics")]
            peer_gauge,
            #[cfg(feature = "metrics")]
            torrent_gauge,
        }
    }

    fn upsert_peer_and_get_response_peers(
        &mut self,
        config: &Config,
        rng: &mut impl Rng,
        valid_until: ValidUntil,
        peer_ip_address: I,
        request: AnnounceRequest,
    ) -> (usize, usize, Vec<ResponsePeer<I>>) {
        self.torrents
            .entry(request.info_hash)
            .or_default()
            .upsert_peer_and_get_response_peers(
                config,
                rng,
                request,
                peer_ip_address,
                valid_until,
                #[cfg(feature = "metrics")]
                &self.peer_gauge,
            )
    }

    fn handle_scrape_request(&mut self, config: &Config, request: ScrapeRequest) -> ScrapeResponse {
        let num_to_take = request
            .info_hashes
            .len()
            .min(config.protocol.max_scrape_torrents);

        let mut response = ScrapeResponse {
            files: BTreeMap::new(),
        };

        for info_hash in request.info_hashes.into_iter().take(num_to_take) {
            let stats = self
                .torrents
                .get(&info_hash)
                .map(|torrent_data| torrent_data.scrape_statistics())
                .unwrap_or(ScrapeStatistics {
                    complete: 0,
                    incomplete: 0,
                    downloaded: 0,
                });

            response.files.insert(info_hash, stats);
        }

        response
    }

    fn clean(
        &mut self,
        config: &Config,
        access_list_cache: &mut AccessListCache,
        now: SecondsSinceServerStart,
    ) {
        let mut total_num_peers = 0;

        self.torrents.retain(|info_hash, torrent_data| {
            if !access_list_cache
                .load()
                .allows(config.access_list.mode, &info_hash.0)
            {
                return false;
            }

            let num_peers = match torrent_data {
                TorrentData::Small(t) => t.clean_and_get_num_peers(now),
                TorrentData::Large(t) => t.clean_and_get_num_peers(now),
            };

            total_num_peers += num_peers as u64;

            num_peers > 0
        });

        self.torrents.shrink_to_fit();

        #[cfg(feature = "metrics")]
        self.peer_gauge.set(total_num_peers as f64);
    }
}

pub enum TorrentData<I: Ip> {
    Small(SmallPeerMap<I>),
    Large(LargePeerMap<I>),
}

impl<I: Ip> TorrentData<I> {
    fn upsert_peer_and_get_response_peers(
        &mut self,
        config: &Config,
        rng: &mut impl Rng,
        request: AnnounceRequest,
        ip_address: I,
        valid_until: ValidUntil,
        #[cfg(feature = "metrics")] peer_gauge: &::metrics::Gauge,
    ) -> (usize, usize, Vec<ResponsePeer<I>>) {
        let max_num_peers_to_take = match request.numwant {
            Some(0) | None => config.protocol.max_peers,
            Some(numwant) => numwant.min(config.protocol.max_peers),
        };

        let status = PeerStatus::from_event_and_bytes_left(request.event, request.bytes_left);

        let peer_map_key = ResponsePeer {
            ip_address,
            port: request.port,
        };

        // Create the response before inserting the peer. This means that we
        // don't have to filter it out from the response peers, and that the
        // reported number of seeders/leechers will not include it
        let (response_data, opt_removed_peer) = match self {
            Self::Small(peer_map) => {
                let opt_removed_peer = peer_map.remove(&peer_map_key);

                let (seeders, leechers) = peer_map.num_seeders_leechers();
                let response_peers = peer_map.extract_response_peers(max_num_peers_to_take);

                // Convert peer map to large variant if it is full and
                // announcing peer is not stopped and will therefore be
                // inserted
                if peer_map.is_full() && status != PeerStatus::Stopped {
                    *self = Self::Large(peer_map.to_large());
                }

                ((seeders, leechers, response_peers), opt_removed_peer)
            }
            Self::Large(peer_map) => {
                let opt_removed_peer = peer_map.remove_peer(&peer_map_key);

                let (seeders, leechers) = peer_map.num_seeders_leechers();
                let response_peers = peer_map.extract_response_peers(rng, max_num_peers_to_take);

                // Try shrinking the map if announcing peer is stopped and
                // will therefore not be inserted
                if status == PeerStatus::Stopped {
                    if let Some(peer_map) = peer_map.try_shrink() {
                        *self = Self::Small(peer_map);
                    }
                }

                ((seeders, leechers, response_peers), opt_removed_peer)
            }
        };

        match status {
            PeerStatus::Leeching | PeerStatus::Seeding => {
                #[cfg(feature = "metrics")]
                if opt_removed_peer.is_none() {
                    peer_gauge.increment(1.0);
                }

                let peer = Peer {
                    is_seeder: status == PeerStatus::Seeding,
                    valid_until,
                };

                match self {
                    Self::Small(peer_map) => peer_map.insert(peer_map_key, peer),
                    Self::Large(peer_map) => peer_map.insert(peer_map_key, peer),
                }
            }
            PeerStatus::Stopped =>
            {
                #[cfg(feature = "metrics")]
                if opt_removed_peer.is_some() {
                    peer_gauge.decrement(1.0);
                }
            }
        };

        response_data
    }

    fn scrape_statistics(&self) -> ScrapeStatistics {
        let (seeders, leechers) = match self {
            Self::Small(peer_map) => peer_map.num_seeders_leechers(),
            Self::Large(peer_map) => peer_map.num_seeders_leechers(),
        };

        ScrapeStatistics {
            complete: seeders,
            incomplete: leechers,
            downloaded: 0,
        }
    }
}

impl<I: Ip> Default for TorrentData<I> {
    fn default() -> Self {
        Self::Small(SmallPeerMap(ArrayVec::default()))
    }
}

/// Store torrents with very few peers without an extra heap allocation
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

    fn clean_and_get_num_peers(&mut self, now: SecondsSinceServerStart) -> usize {
        self.0.retain(|(_, peer)| peer.valid_until.valid(now));

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

    fn clean_and_get_num_peers(&mut self, now: SecondsSinceServerStart) -> usize {
        self.peers.retain(|_, peer| {
            let keep = peer.valid_until.valid(now);

            if (!keep) & peer.is_seeder {
                self.num_seeders -= 1;
            }

            keep
        });

        self.peers.shrink_to_fit();

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

#[derive(Debug, Clone, Copy)]
struct Peer {
    pub valid_until: ValidUntil,
    pub is_seeder: bool,
}

#[derive(PartialEq, Eq, Clone, Copy, Debug)]
enum PeerStatus {
    Seeding,
    Leeching,
    Stopped,
}

impl PeerStatus {
    fn from_event_and_bytes_left(event: AnnounceEvent, bytes_left: usize) -> Self {
        if let AnnounceEvent::Stopped = event {
            Self::Stopped
        } else if bytes_left == 0 {
            Self::Seeding
        } else {
            Self::Leeching
        }
    }
}
