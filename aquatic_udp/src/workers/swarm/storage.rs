use std::net::Ipv4Addr;
use std::net::Ipv6Addr;
use std::sync::Arc;

use aquatic_common::IndexMap;
use aquatic_common::SecondsSinceServerStart;
use aquatic_common::ServerStartInstant;
use aquatic_common::{
    access_list::{create_access_list_cache, AccessListArcSwap, AccessListCache, AccessListMode},
    extract_response_peers, AmortizedIndexMap, ValidUntil,
};

use aquatic_udp_protocol::*;
use hdrhistogram::Histogram;
use heapless::LinearMap;
use rand::prelude::SmallRng;

use crate::common::*;
use crate::config::Config;

use super::create_torrent_scrape_statistics;

#[derive(Clone, Debug, Copy)]
pub struct Peer<I: Ip> {
    pub ip_address: I,
    pub port: Port,
    pub status: PeerStatus,
    pub valid_until: ValidUntil,
}

impl<I: Ip> Peer<I> {
    pub fn to_response_peer(&self) -> ResponsePeer<I> {
        ResponsePeer {
            ip_address: self.ip_address,
            port: self.port,
        }
    }
}

pub type PeerMap<I> = IndexMap<PeerId, Peer<I>>;

pub struct TorrentDataIndexMap<I: Ip> {
    peers: PeerMap<I>,
    num_seeders: usize,
    num_leechers: usize,
}

pub enum TorrentData<I: Ip> {
    Heap(TorrentDataIndexMap<I>),
    Stack(LinearMap<PeerId, Peer<I>, 8>),
}

impl<I: Ip> TorrentData<I> {
    pub fn update_peer(&mut self, peer_id: PeerId, peer: Peer<I>) {
        match self {
            Self::Heap(torrent_data) => {
                let opt_removed_peer = match peer.status {
                    PeerStatus::Leeching => {
                        torrent_data.num_leechers += 1;

                        torrent_data.peers.insert(peer_id, peer)
                    }
                    PeerStatus::Seeding => {
                        torrent_data.num_seeders += 1;

                        torrent_data.peers.insert(peer_id, peer)
                    }
                    PeerStatus::Stopped => torrent_data.peers.remove(&peer_id),
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
            }
            Self::Stack(peers) => {
                if let Err((k, v)) = peers.insert(peer_id, peer) {
                    let mut new_peers = IndexMap::default();

                    new_peers.reserve(peers.len());

                    let mut num_seeders = 0;

                    for (k, v) in peers.iter() {
                        if v.status == PeerStatus::Seeding {
                            num_seeders += 1;
                        }

                        new_peers.insert(*k, *v);
                    }

                    let num_leechers = peers.len() - num_seeders;

                    *self = Self::Heap(TorrentDataIndexMap {
                        peers: new_peers,
                        num_seeders,
                        num_leechers,
                    });

                    self.update_peer(k, v);
                }
            }
        }
    }

    pub fn extract_response_peers(
        &self,
        rng: &mut SmallRng,
        peer_id: PeerId,
        max_num_peers_to_take: usize,
    ) -> Vec<ResponsePeer<I>> {
        match self {
            Self::Heap(torrent_data) => extract_response_peers(
                rng,
                &torrent_data.peers,
                max_num_peers_to_take,
                peer_id,
                Peer::to_response_peer,
            ),
            Self::Stack(peers) => peers
                .iter()
                .filter(|(k, _)| **k != peer_id)
                .take(max_num_peers_to_take)
                .map(|(_, v)| v.to_response_peer())
                .collect(),
        }
    }

    pub fn num_leechers(&self) -> usize {
        match self {
            Self::Heap(TorrentDataIndexMap { num_leechers, .. }) => *num_leechers,
            Self::Stack(peers) => peers
                .values()
                .filter(|p| p.status == PeerStatus::Seeding)
                .count(),
        }
    }

    pub fn num_seeders(&self) -> usize {
        match self {
            Self::Heap(TorrentDataIndexMap { num_seeders, .. }) => *num_seeders,
            Self::Stack(peers) => peers
                .values()
                .filter(|p| p.status != PeerStatus::Seeding)
                .count(),
        }
    }

    pub fn num_peers(&self) -> usize {
        match self {
            Self::Heap(torrent_data) => torrent_data.peers.len(),
            Self::Stack(peers) => peers.len(),
        }
    }

    pub fn scrape_statistics(&self) -> TorrentScrapeStatistics {
        create_torrent_scrape_statistics(
            self.num_seeders().try_into().unwrap_or(i32::MAX),
            self.num_leechers().try_into().unwrap_or(i32::MAX),
        )
    }

    /// Remove inactive peers and reclaim space
    fn clean(&mut self, now: SecondsSinceServerStart) {
        match self {
            Self::Heap(torrent_data) => {
                torrent_data.peers.retain(|_, peer| {
                    if peer.valid_until.valid(now) {
                        true
                    } else {
                        match peer.status {
                            PeerStatus::Seeding => {
                                torrent_data.num_seeders -= 1;
                            }
                            PeerStatus::Leeching => {
                                torrent_data.num_leechers -= 1;
                            }
                            _ => (),
                        };

                        false
                    }
                });

                if !torrent_data.peers.is_empty() {
                    torrent_data.peers.shrink_to_fit();
                }
            }
            Self::Stack(peers) => {
                *peers = peers
                    .iter()
                    .filter(|(_, p)| p.valid_until.valid(now))
                    .map(|(k, v)| (*k, *v))
                    .collect();
            }
        }
    }
}

impl<I: Ip> Default for TorrentData<I> {
    fn default() -> Self {
        Self::Stack(Default::default())
    }
}

#[derive(Default)]
pub struct TorrentMap<I: Ip>(pub AmortizedIndexMap<InfoHash, TorrentData<I>>);

impl<I: Ip> TorrentMap<I> {
    /// Remove forbidden or inactive torrents, reclaim space and return number of remaining peers
    fn clean_and_get_statistics(
        &mut self,
        config: &Config,
        access_list_cache: &mut AccessListCache,
        access_list_mode: AccessListMode,
        now: SecondsSinceServerStart,
    ) -> (usize, Option<Histogram<u64>>) {
        let mut num_peers = 0;

        let mut opt_histogram: Option<Histogram<u64>> = if config.statistics.extended {
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

            torrent.clean(now);

            let n = torrent.num_peers();

            num_peers += n;

            match opt_histogram {
                Some(ref mut histogram) if n != 0 => {
                    let n = n.try_into().expect("Couldn't fit usize into u64");

                    if let Err(err) = histogram.record(n) {
                        ::log::error!("Couldn't record {} to histogram: {:#}", n, err);
                    }
                }
                _ => (),
            }

            n != 0
        });

        self.0.shrink_to_fit();

        (num_peers, opt_histogram)
    }

    pub fn num_torrents(&self) -> usize {
        self.0.len()
    }
}

pub struct TorrentMaps {
    pub ipv4: TorrentMap<Ipv4Addr>,
    pub ipv6: TorrentMap<Ipv6Addr>,
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
    /// Remove forbidden or inactive torrents, reclaim space and return number of remaining peers
    pub fn clean_and_get_statistics(
        &mut self,
        config: &Config,
        access_list: &Arc<AccessListArcSwap>,
        server_start_instant: ServerStartInstant,
    ) -> (
        (usize, Option<Histogram<u64>>),
        (usize, Option<Histogram<u64>>),
    ) {
        let mut cache = create_access_list_cache(access_list);
        let mode = config.access_list.mode;
        let now = server_start_instant.seconds_elapsed();

        let ipv4 = self
            .ipv4
            .clean_and_get_statistics(config, &mut cache, mode, now);
        let ipv6 = self
            .ipv6
            .clean_and_get_statistics(config, &mut cache, mode, now);

        (ipv4, ipv6)
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
            valid_until: ValidUntil::new(ServerStartInstant::new(), 0),
        }
    }

    #[test]
    fn test_extract_response_peers() {
        fn prop(data: (u16, u16)) -> TestResult {
            let gen_num_peers = data.0 as u32;
            let req_num_peers = data.1 as usize;

            let mut peer_map: PeerMap<Ipv4Addr> = Default::default();

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
