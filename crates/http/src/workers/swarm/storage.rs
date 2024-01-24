use std::collections::BTreeMap;
use std::net::{IpAddr, Ipv4Addr, Ipv6Addr};
use std::sync::Arc;

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

#[cfg(feature = "metrics")]
use crate::workers::swarm::WORKER_INDEX;

pub trait Ip: ::std::fmt::Debug + Copy + Eq + ::std::hash::Hash {
    #[cfg(feature = "metrics")]
    fn ip_version_str() -> &'static str;
}

impl Ip for Ipv4Addr {
    #[cfg(feature = "metrics")]
    fn ip_version_str() -> &'static str {
        "4"
    }
}
impl Ip for Ipv6Addr {
    #[cfg(feature = "metrics")]
    fn ip_version_str() -> &'static str {
        "6"
    }
}

#[derive(Default)]
pub struct TorrentMaps {
    pub ipv4: TorrentMap<Ipv4Addr>,
    pub ipv6: TorrentMap<Ipv6Addr>,
}

impl TorrentMaps {
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
                let (seeders, leechers, response_peers) = self
                    .ipv4
                    .entry(request.info_hash)
                    .or_default()
                    .upsert_peer_and_get_response_peers(
                        config,
                        rng,
                        peer_ip_address,
                        request,
                        valid_until,
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
                let (seeders, leechers, response_peers) = self
                    .ipv6
                    .entry(request.info_hash)
                    .or_default()
                    .upsert_peer_and_get_response_peers(
                        config,
                        rng,
                        peer_ip_address,
                        request,
                        valid_until,
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
        let num_to_take = request
            .info_hashes
            .len()
            .min(config.protocol.max_scrape_torrents);

        let mut response = ScrapeResponse {
            files: BTreeMap::new(),
        };

        let peer_ip = peer_addr.get().ip();

        // If request.info_hashes is empty, don't return scrape for all
        // torrents, even though reference server does it. It is too expensive.
        if peer_ip.is_ipv4() {
            for info_hash in request.info_hashes.into_iter().take(num_to_take) {
                if let Some(torrent_data) = self.ipv4.get(&info_hash) {
                    let stats = ScrapeStatistics {
                        complete: torrent_data.num_seeders,
                        downloaded: 0, // No implementation planned
                        incomplete: torrent_data.num_leechers(),
                    };

                    response.files.insert(info_hash, stats);
                }
            }
        } else {
            for info_hash in request.info_hashes.into_iter().take(num_to_take) {
                if let Some(torrent_data) = self.ipv6.get(&info_hash) {
                    let stats = ScrapeStatistics {
                        complete: torrent_data.num_seeders,
                        downloaded: 0, // No implementation planned
                        incomplete: torrent_data.num_leechers(),
                    };

                    response.files.insert(info_hash, stats);
                }
            }
        };

        response
    }

    pub fn clean(
        &mut self,
        config: &Config,
        access_list: &Arc<AccessListArcSwap>,
        server_start_instant: ServerStartInstant,
    ) {
        let mut access_list_cache = create_access_list_cache(access_list);

        let now = server_start_instant.seconds_elapsed();

        Self::clean_torrent_map(config, &mut access_list_cache, &mut self.ipv4, now);
        Self::clean_torrent_map(config, &mut access_list_cache, &mut self.ipv6, now);
    }

    fn clean_torrent_map<I: Ip>(
        config: &Config,
        access_list_cache: &mut AccessListCache,
        torrent_map: &mut TorrentMap<I>,
        now: SecondsSinceServerStart,
    ) {
        let mut total_num_peers = 0;

        torrent_map.retain(|info_hash, torrent_data| {
            if !access_list_cache
                .load()
                .allows(config.access_list.mode, &info_hash.0)
            {
                return false;
            }

            let num_seeders = &mut torrent_data.num_seeders;

            torrent_data.peers.retain(|_, peer| {
                let keep = peer.valid_until.valid(now);

                if (!keep) & peer.seeder {
                    *num_seeders -= 1;
                }

                keep
            });

            total_num_peers += torrent_data.peers.len() as u64;

            !torrent_data.peers.is_empty()
        });

        let total_num_peers = total_num_peers as f64;

        #[cfg(feature = "metrics")]
        ::metrics::gauge!(
            "aquatic_peers",
            total_num_peers,
            "ip_version" => I::ip_version_str(),
            "worker_index" => WORKER_INDEX.with(|index| index.get()).to_string(),
        );

        torrent_map.shrink_to_fit();
    }
}

pub type TorrentMap<I> = IndexMap<InfoHash, TorrentData<I>>;

pub struct TorrentData<I: Ip> {
    peers: IndexMap<ResponsePeer<I>, Peer>,
    num_seeders: usize,
}

impl<I: Ip> Default for TorrentData<I> {
    #[inline]
    fn default() -> Self {
        Self {
            peers: Default::default(),
            num_seeders: 0,
        }
    }
}

impl<I: Ip> TorrentData<I> {
    fn num_leechers(&self) -> usize {
        self.peers.len() - self.num_seeders
    }

    /// Insert/update peer. Return num_seeders, num_leechers and response peers
    fn upsert_peer_and_get_response_peers(
        &mut self,
        config: &Config,
        rng: &mut impl Rng,
        peer_ip_address: I,
        request: AnnounceRequest,
        valid_until: ValidUntil,
    ) -> (usize, usize, Vec<ResponsePeer<I>>) {
        let peer_status =
            PeerStatus::from_event_and_bytes_left(request.event, Some(request.bytes_left));

        let peer_map_key = ResponsePeer {
            ip_address: peer_ip_address,
            port: request.port,
        };

        let opt_removed_peer = self.peers.remove(&peer_map_key);

        if let Some(Peer { seeder: true, .. }) = opt_removed_peer.as_ref() {
            self.num_seeders -= 1;
        }

        let response_peers = match peer_status {
            PeerStatus::Seeding | PeerStatus::Leeching => {
                #[cfg(feature = "metrics")]
                if opt_removed_peer.is_none() {
                    ::metrics::increment_gauge!(
                        "aquatic_peers",
                        1.0,
                        "ip_version" => I::ip_version_str(),
                        "worker_index" => WORKER_INDEX.with(|index| index.get()).to_string(),
                    );
                }

                let max_num_peers_to_take = match request.numwant {
                    Some(0) | None => config.protocol.max_peers,
                    Some(numwant) => numwant.min(config.protocol.max_peers),
                };

                let response_peers = self.extract_response_peers(rng, max_num_peers_to_take);

                let peer = Peer {
                    valid_until,
                    seeder: peer_status == PeerStatus::Seeding,
                };

                self.peers.insert(peer_map_key, peer);

                if peer_status == PeerStatus::Seeding {
                    self.num_seeders += 1;
                }

                response_peers
            }
            PeerStatus::Stopped => {
                #[cfg(feature = "metrics")]
                if opt_removed_peer.is_some() {
                    ::metrics::decrement_gauge!(
                        "aquatic_peers",
                        1.0,
                        "ip_version" => I::ip_version_str(),
                        "worker_index" => WORKER_INDEX.with(|index| index.get()).to_string(),
                    );
                }

                Vec::new()
            }
        };

        (self.num_seeders, self.num_leechers(), response_peers)
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
}

#[derive(Debug, Clone, Copy)]
struct Peer {
    pub valid_until: ValidUntil,
    pub seeder: bool,
}

#[derive(PartialEq, Eq, Clone, Copy, Debug)]
enum PeerStatus {
    Seeding,
    Leeching,
    Stopped,
}

impl PeerStatus {
    /// Determine peer status from announce event and number of bytes left.
    ///
    /// Likely, the last branch will be taken most of the time.
    #[inline]
    fn from_event_and_bytes_left(event: AnnounceEvent, opt_bytes_left: Option<usize>) -> Self {
        if let AnnounceEvent::Stopped = event {
            Self::Stopped
        } else if let Some(0) = opt_bytes_left {
            Self::Seeding
        } else {
            Self::Leeching
        }
    }
}
