use std::net::Ipv4Addr;
use std::net::Ipv6Addr;
use std::sync::Arc;
use std::time::Instant;

use aquatic_common::{
    access_list::{create_access_list_cache, AccessListArcSwap, AccessListCache, AccessListMode},
    extract_response_peers, AmortizedIndexMap, ValidUntil,
};

use aquatic_udp_protocol::*;
use rand::prelude::SmallRng;

use crate::common::*;
use crate::config::Config;

use super::create_torrent_scrape_statistics;

#[derive(Clone, Debug)]
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

pub type PeerMap<I> = AmortizedIndexMap<PeerId, Peer<I>>;

pub struct TorrentData<I: Ip> {
    peers: PeerMap<I>,
    num_seeders: usize,
    num_leechers: usize,
}

impl<I: Ip> TorrentData<I> {
    pub fn update_peer(&mut self, peer_id: PeerId, peer: Peer<I>) {
        let opt_removed_peer = match peer.status {
            PeerStatus::Leeching => {
                self.num_leechers += 1;

                self.peers.insert(peer_id, peer)
            }
            PeerStatus::Seeding => {
                self.num_seeders += 1;

                self.peers.insert(peer_id, peer)
            }
            PeerStatus::Stopped => self.peers.remove(&peer_id),
        };

        match opt_removed_peer.map(|peer| peer.status) {
            Some(PeerStatus::Leeching) => {
                self.num_leechers -= 1;
            }
            Some(PeerStatus::Seeding) => {
                self.num_seeders -= 1;
            }
            _ => {}
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
        self.num_leechers
    }

    pub fn num_seeders(&self) -> usize {
        self.num_seeders
    }

    pub fn scrape_statistics(&self) -> TorrentScrapeStatistics {
        create_torrent_scrape_statistics(
            self.num_seeders.try_into().unwrap_or(i32::MAX),
            self.num_leechers.try_into().unwrap_or(i32::MAX),
        )
    }

    /// Remove inactive peers and reclaim space
    fn clean(&mut self, now: Instant) {
        self.peers.retain(|_, peer| {
            if peer.valid_until.0 > now {
                true
            } else {
                match peer.status {
                    PeerStatus::Seeding => {
                        self.num_seeders -= 1;
                    }
                    PeerStatus::Leeching => {
                        self.num_leechers -= 1;
                    }
                    _ => (),
                };

                false
            }
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
            num_leechers: 0,
        }
    }
}

#[derive(Default)]
pub struct TorrentMap<I: Ip>(pub AmortizedIndexMap<InfoHash, TorrentData<I>>);

impl<I: Ip> TorrentMap<I> {
    /// Remove forbidden or inactive torrents, reclaim space and return number of remaining peers
    fn clean_and_get_num_peers(
        &mut self,
        access_list_cache: &mut AccessListCache,
        access_list_mode: AccessListMode,
        now: Instant,
    ) -> usize {
        let mut num_peers = 0;

        self.0.retain(|info_hash, torrent| {
            if !access_list_cache
                .load()
                .allows(access_list_mode, &info_hash.0)
            {
                return false;
            }

            torrent.clean(now);

            num_peers += torrent.peers.len();

            !torrent.peers.is_empty()
        });

        self.0.shrink_to_fit();

        num_peers
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
    pub fn clean_and_get_num_peers(
        &mut self,
        config: &Config,
        access_list: &Arc<AccessListArcSwap>,
    ) -> (usize, usize) {
        let mut cache = create_access_list_cache(access_list);
        let mode = config.access_list.mode;
        let now = Instant::now();

        let ipv4 = self.ipv4.clean_and_get_num_peers(&mut cache, mode, now);
        let ipv6 = self.ipv6.clean_and_get_num_peers(&mut cache, mode, now);

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
            valid_until: ValidUntil::new(0),
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
