use std::borrow::Cow;
use std::collections::BTreeMap;
use std::hash::Hash;
use std::io::Write;
use std::mem::size_of;
use std::net::{SocketAddr, SocketAddrV4};
use std::sync::atomic::AtomicUsize;
use std::sync::Arc;

use crossbeam_channel::{Sender, TrySendError};

use aquatic_common::access_list::AccessListArcSwap;
use aquatic_common::CanonicalSocketAddr;
use aquatic_udp_protocol::*;
use hdrhistogram::Histogram;
use thingbuf::mpsc::blocking::SendRef;

use crate::config::Config;

pub const BUFFER_SIZE: usize = 8192;

#[derive(PartialEq, Eq, Clone, Debug)]
pub enum CowResponse<'a> {
    Connect(Cow<'a, ConnectResponse>),
    AnnounceIpv4(Cow<'a, AnnounceResponse<Ipv4AddrBytes>>),
    AnnounceIpv6(Cow<'a, AnnounceResponse<Ipv6AddrBytes>>),
    Scrape(Cow<'a, ScrapeResponse>),
    Error(Cow<'a, ErrorResponse>),
}

impl From<Response> for CowResponse<'_> {
    fn from(value: Response) -> Self {
        match value {
            Response::AnnounceIpv4(r) => Self::AnnounceIpv4(Cow::Owned(r)),
            Response::AnnounceIpv6(r) => Self::AnnounceIpv6(Cow::Owned(r)),
            Response::Connect(r) => Self::Connect(Cow::Owned(r)),
            Response::Scrape(r) => Self::Scrape(Cow::Owned(r)),
            Response::Error(r) => Self::Error(Cow::Owned(r)),
        }
    }
}

impl<'a> CowResponse<'a> {
    pub fn into_owned(self) -> Response {
        match self {
            CowResponse::Connect(r) => Response::Connect(r.into_owned()),
            CowResponse::AnnounceIpv4(r) => Response::AnnounceIpv4(r.into_owned()),
            CowResponse::AnnounceIpv6(r) => Response::AnnounceIpv6(r.into_owned()),
            CowResponse::Scrape(r) => Response::Scrape(r.into_owned()),
            CowResponse::Error(r) => Response::Error(r.into_owned()),
        }
    }

    #[inline]
    pub fn write(&self, bytes: &mut impl Write) -> Result<(), ::std::io::Error> {
        match self {
            Self::Connect(r) => r.write(bytes),
            Self::AnnounceIpv4(r) => r.write(bytes),
            Self::AnnounceIpv6(r) => r.write(bytes),
            Self::Scrape(r) => r.write(bytes),
            Self::Error(r) => r.write(bytes),
        }
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

pub enum ConnectedResponseKind {
    AnnounceIpv4,
    AnnounceIpv6,
    Scrape,
}

pub struct ConnectedResponseWithAddr {
    pub kind: ConnectedResponseKind,
    pub announce_ipv4: AnnounceResponse<Ipv4AddrBytes>,
    pub announce_ipv6: AnnounceResponse<Ipv6AddrBytes>,
    pub scrape: PendingScrapeResponse,
    pub addr: CanonicalSocketAddr,
}

impl ConnectedResponseWithAddr {
    pub fn estimated_max_size(config: &Config) -> usize {
        size_of::<Self>()
            + config.protocol.max_response_peers
                * (size_of::<ResponsePeer<Ipv4AddrBytes>>()
                    + size_of::<ResponsePeer<Ipv6AddrBytes>>())
    }
}

pub struct Recycler;

impl thingbuf::Recycle<ConnectedResponseWithAddr> for Recycler {
    fn new_element(&self) -> ConnectedResponseWithAddr {
        ConnectedResponseWithAddr {
            kind: ConnectedResponseKind::AnnounceIpv4,
            announce_ipv4: AnnounceResponse::empty(),
            announce_ipv6: AnnounceResponse::empty(),
            scrape: PendingScrapeResponse {
                slab_key: 0,
                torrent_stats: Default::default(),
            },
            addr: CanonicalSocketAddr::new(SocketAddr::V4(SocketAddrV4::new(0.into(), 0))),
        }
    }
    fn recycle(&self, element: &mut ConnectedResponseWithAddr) {
        element.announce_ipv4.peers.clear();
        element.announce_ipv6.peers.clear();
        element.scrape.torrent_stats.clear();
        element.addr = CanonicalSocketAddr::new(SocketAddr::V4(SocketAddrV4::new(0.into(), 0)));
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
    senders: Vec<thingbuf::mpsc::blocking::Sender<ConnectedResponseWithAddr, Recycler>>,
    to_any_last_index_picked: usize,
}

impl ConnectedResponseSender {
    pub fn new(
        senders: Vec<thingbuf::mpsc::blocking::Sender<ConnectedResponseWithAddr, Recycler>>,
    ) -> Self {
        Self {
            senders,
            to_any_last_index_picked: 0,
        }
    }

    pub fn try_send_ref_to(
        &self,
        index: SocketWorkerIndex,
    ) -> Result<SendRef<ConnectedResponseWithAddr>, thingbuf::mpsc::errors::TrySendError> {
        self.senders[index.0].try_send_ref()
    }

    pub fn send_ref_to(
        &self,
        index: SocketWorkerIndex,
    ) -> Result<SendRef<ConnectedResponseWithAddr>, thingbuf::mpsc::errors::Closed> {
        self.senders[index.0].send_ref()
    }

    pub fn send_ref_to_any(
        &mut self,
    ) -> Result<SendRef<ConnectedResponseWithAddr>, thingbuf::mpsc::errors::Closed> {
        let start = self.to_any_last_index_picked + 1;

        for i in (start..start + self.senders.len()).map(|i| i % self.senders.len()) {
            if let Ok(sender) = self.senders[i].try_send_ref() {
                self.to_any_last_index_picked = i;

                return Ok(sender);
            }
        }

        self.to_any_last_index_picked = start % self.senders.len();
        self.send_ref_to(SocketWorkerIndex(self.to_any_last_index_picked))
    }
}

pub type ConnectedResponseReceiver =
    thingbuf::mpsc::blocking::Receiver<ConnectedResponseWithAddr, Recycler>;

#[derive(PartialEq, Eq, Hash, Clone, Copy, Debug)]
pub enum PeerStatus {
    Seeding,
    Leeching,
    Stopped,
}

impl PeerStatus {
    /// Determine peer status from announce event and number of bytes left.
    ///
    /// Likely, the last branch will be taken most of the time.
    #[inline]
    pub fn from_event_and_bytes_left(event: AnnounceEvent, bytes_left: NumberOfBytes) -> Self {
        if event == AnnounceEvent::Stopped {
            Self::Stopped
        } else if bytes_left.0.get() == 0 {
            Self::Seeding
        } else {
            Self::Leeching
        }
    }
}

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
        ::std::iter::repeat_with(|| AtomicUsize::default())
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

    #[test]
    fn test_peer_status_from_event_and_bytes_left() {
        use crate::common::*;

        use PeerStatus::*;

        let f = PeerStatus::from_event_and_bytes_left;

        assert_eq!(Stopped, f(AnnounceEvent::Stopped, NumberOfBytes::new(0)));
        assert_eq!(Stopped, f(AnnounceEvent::Stopped, NumberOfBytes::new(1)));

        assert_eq!(Seeding, f(AnnounceEvent::Started, NumberOfBytes::new(0)));
        assert_eq!(Leeching, f(AnnounceEvent::Started, NumberOfBytes::new(1)));

        assert_eq!(Seeding, f(AnnounceEvent::Completed, NumberOfBytes::new(0)));
        assert_eq!(Leeching, f(AnnounceEvent::Completed, NumberOfBytes::new(1)));

        assert_eq!(Seeding, f(AnnounceEvent::None, NumberOfBytes::new(0)));
        assert_eq!(Leeching, f(AnnounceEvent::None, NumberOfBytes::new(1)));
    }

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

        response.write(&mut buf).unwrap();

        println!("Buffer len: {}", buf.len());

        assert!(buf.len() <= BUFFER_SIZE);
    }
}
