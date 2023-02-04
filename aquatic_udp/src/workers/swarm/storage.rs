use std::net::Ipv4Addr;
use std::net::Ipv6Addr;
use std::sync::atomic::Ordering;

use aquatic_common::IndexMap;
use aquatic_common::SecondsSinceServerStart;
use aquatic_common::ServerStartInstant;
use aquatic_common::{
    access_list::{create_access_list_cache, AccessListCache, AccessListMode},
    extract_response_peers, ValidUntil,
};

use aquatic_udp_protocol::*;
use rand::prelude::SmallRng;

use crate::common::*;
use crate::config::Config;

use super::create_torrent_scrape_statistics;

pub struct ProtocolTorrentMaps {
    pub ipv4: TorrentMaps<Ipv4Addr>,
    pub ipv6: TorrentMaps<Ipv6Addr>,
}

impl ProtocolTorrentMaps {
    pub fn new(config: &Config, start_instant: ServerStartInstant) -> Self {
        Self {
            ipv4: TorrentMaps::new(config, start_instant),
            ipv6: TorrentMaps::new(config, start_instant),
        }
    }

    /// Remove forbidden or inactive torrents, reclaim space and update peer count statistics
    pub fn clean_and_update_peer_statistics(
        &mut self,
        config: &Config,
        shared_state: &State,
        worker_index: SwarmWorkerIndex,
        server_start_instant: ServerStartInstant,
    ) {
        let mut cache = create_access_list_cache(&shared_state.access_list);
        let mode = config.access_list.mode;
        let now = server_start_instant.seconds_elapsed();

        let mut ipv4_peers_removed = 0;
        let mut ipv6_peers_removed = 0;

        for map in self.ipv4.0.iter_mut() {
            if map.needs_cleaning(now) {
                ipv4_peers_removed +=
                    map.clean_and_get_num_removed_peers(config, &mut cache, mode, now);
            }
        }
        for map in self.ipv6.0.iter_mut() {
            if map.needs_cleaning(now) {
                ipv6_peers_removed +=
                    map.clean_and_get_num_removed_peers(config, &mut cache, mode, now);
            }
        }

        if config.statistics.active() {
            shared_state.statistics_ipv4.peers[worker_index.0]
                .fetch_sub(ipv4_peers_removed, Ordering::Relaxed);
            shared_state.statistics_ipv6.peers[worker_index.0]
                .fetch_sub(ipv6_peers_removed, Ordering::Relaxed);
        }
    }
}

pub struct TorrentMaps<I>(Vec<TorrentMap<I>>);

impl<I: Ip> TorrentMaps<I> {
    fn new(config: &Config, start_instant: ServerStartInstant) -> Self {
        let next_cleaning_due =
            ValidUntil::new(start_instant, config.cleaning.torrent_cleaning_interval);

        let maps = ::std::iter::repeat_with(|| TorrentMap {
            torrents: Default::default(),
            next_cleaning_due,
        })
        .take(2usize.pow(config.cleaning.num_torrent_maps_pow2 as u32))
        .collect();

        Self(maps)
    }

    pub fn get_map(&mut self, info_hash_bytes: [u8; 20]) -> &mut TorrentMap<I> {
        let index =
            (info_hash_bytes[0] as usize + (info_hash_bytes[1] as usize >> 8)).min(self.0.len());

        self.0.get_mut(index).unwrap()
    }

    pub fn total_num_torrents(&self) -> usize {
        self.0.iter().map(|map| map.num_torrents()).sum()
    }
}

pub struct TorrentMap<I> {
    pub torrents: IndexMap<InfoHash, TorrentData<I>>,
    next_cleaning_due: ValidUntil,
}

impl<I: Ip> TorrentMap<I> {
    /// Remove forbidden or inactive torrents, reclaim space and return number of removed peers
    pub fn clean_and_get_num_removed_peers(
        &mut self,
        config: &Config,
        access_list_cache: &mut AccessListCache,
        access_list_mode: AccessListMode,
        now: SecondsSinceServerStart,
    ) -> usize {
        let mut num_peers_before = 0;
        let mut num_peers_after = 0;

        self.torrents.retain(|info_hash, torrent| {
            num_peers_before += torrent.peers.len();

            if !access_list_cache
                .load()
                .allows(access_list_mode, &info_hash.0)
            {
                return false;
            }

            torrent.clean(now);

            num_peers_after += torrent.peers.len();

            !torrent.peers.is_empty()
        });

        self.torrents.shrink_to_fit();

        self.next_cleaning_due =
            ValidUntil::new_with_now(now, config.cleaning.torrent_cleaning_interval);

        num_peers_before - num_peers_after
    }

    pub fn num_torrents(&self) -> usize {
        self.torrents.len()
    }

    pub fn needs_cleaning(&self, now: SecondsSinceServerStart) -> bool {
        !self.next_cleaning_due.valid(now)
    }
}

pub struct TorrentData<I> {
    peers: IndexMap<PeerId, Peer<I>>,
    num_seeders: usize,
}

impl<I: Ip> TorrentData<I> {
    pub fn update_peer(
        &mut self,
        config: &Config,
        statistics: &Statistics,
        worker_index: SwarmWorkerIndex,
        peer_id: PeerId,
        ip_address: I,
        port: Port,
        status: PeerStatus,
        valid_until: ValidUntil,
    ) {
        let opt_removed_peer = match status {
            PeerStatus::Leeching => {
                let peer = Peer {
                    ip_address,
                    port,
                    is_seeder: false,
                    valid_until,
                };

                self.peers.insert(peer_id, peer)
            }
            PeerStatus::Seeding => {
                let peer = Peer {
                    ip_address,
                    port,
                    is_seeder: true,
                    valid_until,
                };

                self.num_seeders += 1;

                self.peers.insert(peer_id, peer)
            }
            PeerStatus::Stopped => self.peers.remove(&peer_id),
        };

        if let Some(Peer {
            is_seeder: true, ..
        }) = opt_removed_peer
        {
            self.num_seeders -= 1;
        }

        if config.statistics.active() {
            match (status, opt_removed_peer) {
                // Peer count increased by 1
                (PeerStatus::Leeching | PeerStatus::Seeding, None) => {
                    statistics.peers[worker_index.0].fetch_add(1, Ordering::Relaxed);
                }
                // Peer count decreased by 1
                (PeerStatus::Stopped, Some(_)) => {
                    statistics.peers[worker_index.0].fetch_sub(1, Ordering::Relaxed);
                }
                // Peer count unchanged
                (PeerStatus::Stopped, None)
                | (PeerStatus::Leeching | PeerStatus::Seeding, Some(_)) => (),
            }
        }
    }

    pub fn extract_response_peers(
        &self,
        rng: &mut SmallRng,
        peer_id: PeerId,
        max_num_peers_to_take: usize,
    ) -> Vec<ResponsePeer<I>> {
        extract_response_peers(
            rng,
            &self.peers,
            max_num_peers_to_take,
            peer_id,
            Peer::to_response_peer,
        )
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
    fn clean(&mut self, now: SecondsSinceServerStart) {
        self.peers.retain(|_, peer| {
            let keep = peer.valid_until.valid(now);

            if (!keep) & peer.is_seeder {
                self.num_seeders -= 1;
            }

            keep
        });

        if !self.peers.is_empty() {
            self.peers.shrink_to_fit();
        }
    }
}

impl<I> Default for TorrentData<I> {
    fn default() -> Self {
        Self {
            peers: Default::default(),
            num_seeders: 0,
        }
    }
}

#[derive(Clone, Debug)]
struct Peer<I> {
    ip_address: I,
    port: Port,
    is_seeder: bool,
    valid_until: ValidUntil,
}

impl<I: Ip> Peer<I> {
    fn to_response_peer(&self) -> ResponsePeer<I> {
        ResponsePeer {
            ip_address: self.ip_address,
            port: self.port,
        }
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
            is_seeder: false,
            valid_until: ValidUntil::new(ServerStartInstant::new(), 0),
        }
    }

    #[test]
    fn test_extract_response_peers() {
        fn prop(data: (u16, u16)) -> TestResult {
            let gen_num_peers = data.0 as u32;
            let req_num_peers = data.1 as usize;

            let mut peer_map: IndexMap<PeerId, Peer<Ipv4Addr>> = Default::default();

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
