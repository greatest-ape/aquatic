use std::net::Ipv4Addr;
use std::net::Ipv6Addr;
use std::sync::atomic::Ordering;
use std::sync::Arc;

use aquatic_common::IndexMap;
use aquatic_common::SecondsSinceServerStart;
use aquatic_common::ServerStartInstant;
use aquatic_common::{
    access_list::{create_access_list_cache, AccessListArcSwap, AccessListCache, AccessListMode},
    extract_response_peers, ValidUntil,
};

use aquatic_udp_protocol::*;
use crossbeam_channel::Sender;
use hdrhistogram::Histogram;
use rand::prelude::SmallRng;

use crate::common::*;
use crate::config::Config;

use super::create_torrent_scrape_statistics;

#[derive(Clone, Debug)]
struct Peer<I: Ip> {
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

type PeerMap<I> = IndexMap<PeerId, Peer<I>>;

pub struct TorrentData<I: Ip> {
    peers: PeerMap<I>,
    num_seeders: usize,
}

impl<I: Ip> TorrentData<I> {
    pub fn update_peer(
        &mut self,
        config: &Config,
        statistics_sender: &Sender<StatisticsMessage>,
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

        if config.statistics.peer_clients {
            match (status, opt_removed_peer.is_some()) {
                // We added a new peer
                (PeerStatus::Leeching | PeerStatus::Seeding, false) => {
                    if let Err(_) =
                        statistics_sender.try_send(StatisticsMessage::PeerAdded(peer_id))
                    {
                        // Should never happen in practice
                        ::log::error!("Couldn't send StatisticsMessage::PeerAdded");
                    }
                }
                // We removed an existing peer
                (PeerStatus::Stopped, true) => {
                    if let Err(_) =
                        statistics_sender.try_send(StatisticsMessage::PeerRemoved(peer_id))
                    {
                        // Should never happen in practice
                        ::log::error!("Couldn't send StatisticsMessage::PeerRemoved");
                    }
                }
                _ => (),
            }
        }

        if let Some(Peer {
            is_seeder: true, ..
        }) = opt_removed_peer
        {
            self.num_seeders -= 1;
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

    #[cfg(feature = "full-scrapes")]
    pub fn full_scrape(&self) -> Vec<aquatic_common::full_scrape::FullScrapeStatistics> {
        self.0
            .iter()
            .map(
                |(info_hash, torrent)| aquatic_common::full_scrape::FullScrapeStatistics {
                    info_hash: info_hash.0,
                    seeders: torrent.num_seeders(),
                    leechers: torrent.num_leechers(),
                },
            )
            .collect()
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
