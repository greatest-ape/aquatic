use std::iter::repeat_with;
use std::sync::atomic::AtomicUsize;
use std::sync::Arc;

use aquatic_common::access_list::AccessListArcSwap;
use aquatic_common::ServerStartInstant;
use aquatic_udp_protocol::*;
use crossbeam_utils::CachePadded;
use hdrhistogram::Histogram;

use crate::config::Config;
use crate::swarm::TorrentMaps;

pub const BUFFER_SIZE: usize = 8192;

#[derive(Clone, Copy, Debug)]
pub enum IpVersion {
    V4,
    V6,
}

#[cfg(feature = "prometheus")]
impl IpVersion {
    pub fn prometheus_str(&self) -> &'static str {
        match self {
            Self::V4 => "4",
            Self::V6 => "6",
        }
    }
}

#[derive(Clone)]
pub struct Statistics {
    pub socket: Vec<CachePaddedArc<IpVersionStatistics<SocketWorkerStatistics>>>,
    pub swarm: CachePaddedArc<IpVersionStatistics<SwarmWorkerStatistics>>,
}

impl Statistics {
    pub fn new(config: &Config) -> Self {
        Self {
            socket: repeat_with(Default::default)
                .take(config.socket_workers)
                .collect(),
            swarm: Default::default(),
        }
    }
}

#[derive(Default)]
pub struct IpVersionStatistics<T> {
    pub ipv4: T,
    pub ipv6: T,
}

impl<T> IpVersionStatistics<T> {
    pub fn by_ip_version(&self, ip_version: IpVersion) -> &T {
        match ip_version {
            IpVersion::V4 => &self.ipv4,
            IpVersion::V6 => &self.ipv6,
        }
    }
}

#[derive(Default)]
pub struct SocketWorkerStatistics {
    pub requests: AtomicUsize,
    pub responses_connect: AtomicUsize,
    pub responses_announce: AtomicUsize,
    pub responses_scrape: AtomicUsize,
    pub responses_error: AtomicUsize,
    pub bytes_received: AtomicUsize,
    pub bytes_sent: AtomicUsize,
}

pub type CachePaddedArc<T> = CachePadded<Arc<CachePadded<T>>>;

#[derive(Default)]
pub struct SwarmWorkerStatistics {
    pub torrents: AtomicUsize,
    pub peers: AtomicUsize,
}

pub enum StatisticsMessage {
    Ipv4PeerHistogram(Histogram<u64>),
    Ipv6PeerHistogram(Histogram<u64>),
    PeerAdded(PeerId),
    PeerRemoved(PeerId),
}

#[derive(Clone)]
pub struct State {
    pub access_list: Arc<AccessListArcSwap>,
    pub torrent_maps: TorrentMaps,
    pub server_start_instant: ServerStartInstant,
}

impl Default for State {
    fn default() -> Self {
        Self {
            access_list: Arc::new(AccessListArcSwap::default()),
            torrent_maps: TorrentMaps::default(),
            server_start_instant: ServerStartInstant::new(),
        }
    }
}

#[cfg(test)]
mod tests {
    use std::{net::Ipv6Addr, num::NonZeroU16};

    use crate::config::Config;

    use super::*;

    // Assumes that announce response with maximum amount of ipv6 peers will
    // be the longest
    #[test]
    fn test_buffer_size() {
        use aquatic_udp_protocol::*;

        let config = Config::default();

        let peers = ::std::iter::repeat(ResponsePeer {
            ip_address: Ipv6AddrBytes(Ipv6Addr::new(1, 1, 1, 1, 1, 1, 1, 1).octets()),
            port: Port::new(NonZeroU16::new(1).unwrap()),
        })
        .take(config.protocol.max_response_peers)
        .collect();

        let response = Response::AnnounceIpv6(AnnounceResponse {
            fixed: AnnounceResponseFixedData {
                transaction_id: TransactionId::new(1),
                announce_interval: AnnounceInterval::new(1),
                seeders: NumberOfPeers::new(1),
                leechers: NumberOfPeers::new(1),
            },
            peers,
        });

        let mut buf = Vec::new();

        response.write_bytes(&mut buf).unwrap();

        println!("Buffer len: {}", buf.len());

        assert!(buf.len() <= BUFFER_SIZE);
    }
}
