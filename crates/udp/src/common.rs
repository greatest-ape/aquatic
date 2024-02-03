use std::collections::BTreeMap;
use std::hash::Hash;
use std::iter::repeat_with;
use std::sync::atomic::AtomicUsize;
use std::sync::Arc;

use crossbeam_channel::{Receiver, SendError, Sender, TrySendError};

use aquatic_common::access_list::AccessListArcSwap;
use aquatic_common::{CanonicalSocketAddr, ServerStartInstant};
use aquatic_udp_protocol::*;
use crossbeam_utils::CachePadded;
use hdrhistogram::Histogram;

use crate::config::Config;

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

#[derive(Clone, Copy, Debug)]
pub struct SocketWorkerIndex(pub usize);

#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub struct SwarmWorkerIndex(pub usize);

impl SwarmWorkerIndex {
    pub fn from_info_hash(config: &Config, info_hash: InfoHash) -> Self {
        Self(info_hash.0[0] as usize % config.swarm_workers)
    }
}

#[derive(Debug)]
pub struct PendingScrapeRequest {
    pub slab_key: usize,
    pub info_hashes: BTreeMap<usize, InfoHash>,
}

#[derive(Debug)]
pub struct PendingScrapeResponse {
    pub slab_key: usize,
    pub torrent_stats: BTreeMap<usize, TorrentScrapeStatistics>,
}

#[derive(Debug)]
pub enum ConnectedRequest {
    Announce(AnnounceRequest),
    Scrape(PendingScrapeRequest),
}

#[derive(Debug)]
pub enum ConnectedResponse {
    AnnounceIpv4(AnnounceResponse<Ipv4AddrBytes>),
    AnnounceIpv6(AnnounceResponse<Ipv6AddrBytes>),
    Scrape(PendingScrapeResponse),
}

pub struct ConnectedRequestSender {
    index: SocketWorkerIndex,
    senders: Vec<Sender<(SocketWorkerIndex, ConnectedRequest, CanonicalSocketAddr)>>,
}

impl ConnectedRequestSender {
    pub fn new(
        index: SocketWorkerIndex,
        senders: Vec<Sender<(SocketWorkerIndex, ConnectedRequest, CanonicalSocketAddr)>>,
    ) -> Self {
        Self { index, senders }
    }

    pub fn try_send_to(
        &self,
        index: SwarmWorkerIndex,
        request: ConnectedRequest,
        addr: CanonicalSocketAddr,
    ) -> Result<(), (SwarmWorkerIndex, ConnectedRequest, CanonicalSocketAddr)> {
        match self.senders[index.0].try_send((self.index, request, addr)) {
            Ok(()) => Ok(()),
            Err(TrySendError::Full(r)) => Err((index, r.1, r.2)),
            Err(TrySendError::Disconnected(_)) => {
                panic!("Request channel {} is disconnected", index.0);
            }
        }
    }
}

pub struct ConnectedResponseSender {
    senders: Vec<Sender<(CanonicalSocketAddr, ConnectedResponse)>>,
    to_any_last_index_picked: usize,
}

impl ConnectedResponseSender {
    pub fn new(senders: Vec<Sender<(CanonicalSocketAddr, ConnectedResponse)>>) -> Self {
        Self {
            senders,
            to_any_last_index_picked: 0,
        }
    }

    pub fn try_send_to(
        &self,
        index: SocketWorkerIndex,
        addr: CanonicalSocketAddr,
        response: ConnectedResponse,
    ) -> Result<(), TrySendError<(CanonicalSocketAddr, ConnectedResponse)>> {
        self.senders[index.0].try_send((addr, response))
    }

    pub fn send_to(
        &self,
        index: SocketWorkerIndex,
        addr: CanonicalSocketAddr,
        response: ConnectedResponse,
    ) -> Result<(), SendError<(CanonicalSocketAddr, ConnectedResponse)>> {
        self.senders[index.0].send((addr, response))
    }

    pub fn send_to_any(
        &mut self,
        addr: CanonicalSocketAddr,
        response: ConnectedResponse,
    ) -> Result<(), SendError<(CanonicalSocketAddr, ConnectedResponse)>> {
        let start = self.to_any_last_index_picked + 1;

        let mut message = Some((addr, response));

        for i in (start..start + self.senders.len()).map(|i| i % self.senders.len()) {
            match self.senders[i].try_send(message.take().unwrap()) {
                Ok(()) => {
                    self.to_any_last_index_picked = i;

                    return Ok(());
                }
                Err(TrySendError::Full(msg)) => {
                    message = Some(msg);
                }
                Err(TrySendError::Disconnected(_)) => {
                    panic!("ConnectedResponseReceiver disconnected");
                }
            }
        }

        let (addr, response) = message.unwrap();

        self.to_any_last_index_picked = start % self.senders.len();
        self.send_to(
            SocketWorkerIndex(self.to_any_last_index_picked),
            addr,
            response,
        )
    }
}

pub type ConnectedResponseReceiver = Receiver<(CanonicalSocketAddr, ConnectedResponse)>;

#[derive(Clone)]
pub struct Statistics {
    pub socket: Vec<CachePaddedArc<IpVersionStatistics<SocketWorkerStatistics>>>,
    pub swarm: Vec<CachePaddedArc<IpVersionStatistics<SwarmWorkerStatistics>>>,
}

impl Statistics {
    pub fn new(config: &Config) -> Self {
        Self {
            socket: repeat_with(Default::default)
                .take(config.socket_workers)
                .collect(),
            swarm: repeat_with(Default::default)
                .take(config.swarm_workers)
                .collect(),
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
    pub server_start_instant: ServerStartInstant,
}

impl Default for State {
    fn default() -> Self {
        Self {
            access_list: Arc::new(AccessListArcSwap::default()),
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
