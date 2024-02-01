use std::collections::BTreeMap;
use std::hash::Hash;
use std::sync::atomic::AtomicUsize;
use std::sync::Arc;

use crossbeam_channel::{Receiver, SendError, Sender, TrySendError};

use aquatic_common::access_list::AccessListArcSwap;
use aquatic_common::CanonicalSocketAddr;
use aquatic_udp_protocol::*;
use hdrhistogram::Histogram;

use crate::config::Config;

pub const BUFFER_SIZE: usize = 8192;

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

#[derive(Clone, Copy, Debug)]
pub struct SocketWorkerIndex(pub usize);

#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub struct SwarmWorkerIndex(pub usize);

impl SwarmWorkerIndex {
    pub fn from_info_hash(config: &Config, info_hash: InfoHash) -> Self {
        Self(info_hash.0[0] as usize % config.swarm_workers)
    }
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

    pub fn any_full(&self) -> bool {
        self.senders.iter().any(|sender| sender.is_full())
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

pub enum StatisticsMessage {
    Ipv4PeerHistogram(Histogram<u64>),
    Ipv6PeerHistogram(Histogram<u64>),
    PeerAdded(PeerId),
    PeerRemoved(PeerId),
}

pub struct Statistics {
    pub requests_received: AtomicUsize,
    pub responses_sent_connect: AtomicUsize,
    pub responses_sent_announce: AtomicUsize,
    pub responses_sent_scrape: AtomicUsize,
    pub responses_sent_error: AtomicUsize,
    pub bytes_received: AtomicUsize,
    pub bytes_sent: AtomicUsize,
    pub torrents: Vec<AtomicUsize>,
    pub peers: Vec<AtomicUsize>,
}

impl Statistics {
    pub fn new(num_swarm_workers: usize) -> Self {
        Self {
            requests_received: Default::default(),
            responses_sent_connect: Default::default(),
            responses_sent_announce: Default::default(),
            responses_sent_scrape: Default::default(),
            responses_sent_error: Default::default(),
            bytes_received: Default::default(),
            bytes_sent: Default::default(),
            torrents: Self::create_atomic_usize_vec(num_swarm_workers),
            peers: Self::create_atomic_usize_vec(num_swarm_workers),
        }
    }

    fn create_atomic_usize_vec(len: usize) -> Vec<AtomicUsize> {
        ::std::iter::repeat_with(AtomicUsize::default)
            .take(len)
            .collect()
    }
}

#[derive(Clone)]
pub struct State {
    pub access_list: Arc<AccessListArcSwap>,
    pub statistics_ipv4: Arc<Statistics>,
    pub statistics_ipv6: Arc<Statistics>,
}

impl State {
    pub fn new(num_swarm_workers: usize) -> Self {
        Self {
            access_list: Arc::new(AccessListArcSwap::default()),
            statistics_ipv4: Arc::new(Statistics::new(num_swarm_workers)),
            statistics_ipv6: Arc::new(Statistics::new(num_swarm_workers)),
        }
    }
}

#[cfg(test)]
mod tests {
    use std::net::Ipv6Addr;

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
            port: Port::new(1),
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
