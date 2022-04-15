use std::net::Ipv4Addr;
use std::net::Ipv6Addr;
use std::sync::Arc;
use std::time::Instant;

use aquatic_common::{
    access_list::{create_access_list_cache, AccessListArcSwap, AccessListCache, AccessListMode},
    AmortizedIndexMap, ValidUntil,
};

use aquatic_udp_protocol::*;

use crate::common::*;
use crate::config::Config;

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
    pub peers: PeerMap<I>,
    pub num_seeders: usize,
    pub num_leechers: usize,
}

impl<I: Ip> TorrentData<I> {
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
