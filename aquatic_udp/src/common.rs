use std::collections::BTreeMap;
use std::hash::Hash;
use std::net::{IpAddr, Ipv4Addr, Ipv6Addr};
use std::sync::atomic::AtomicUsize;
use std::sync::Arc;
use std::time::Instant;

use anyhow::Context;
use crossbeam_channel::{Sender, TrySendError};
use getrandom::getrandom;

use aquatic_common::access_list::AccessListArcSwap;
use aquatic_common::CanonicalSocketAddr;
use aquatic_udp_protocol::*;

use crate::config::Config;

pub const MAX_PACKET_SIZE: usize = 8192;

/// HMAC (BLAKE3) based ConnectionID creator and validator
///
/// Structure of created ConnectionID (bytes making up inner i64):
/// - &[0..4]: connection expiration time as number of seconds after
///   ConnectionValidator instance was created, encoded as u32 bytes
/// - &[4..8]: truncated keyed BLAKE3 hash of above 4 bytes and octets of
///   client IP address
#[derive(Clone)]
pub struct ConnectionValidator {
    start_time: Instant,
    max_connection_age: u32,
    hmac: blake3::Hasher,
}

impl ConnectionValidator {
    /// Create new instance. Must be created once and cloned if used in several
    /// threads.
    pub fn new(config: &Config) -> anyhow::Result<Self> {
        let mut key = [0; 32];

        getrandom(&mut key)
            .with_context(|| "Couldn't get random bytes for ConnectionValidator key")?;

        let hmac = blake3::Hasher::new_keyed(&key);

        Ok(Self {
            hmac,
            start_time: Instant::now(),
            max_connection_age: config.cleaning.max_connection_age,
        })
    }

    pub fn create_connection_id(&mut self, source_addr: CanonicalSocketAddr) -> ConnectionId {
        let valid_until =
            (self.start_time.elapsed().as_secs() as u32 + self.max_connection_age).to_ne_bytes();

        self.create_connection_id_inner(valid_until, source_addr)
    }

    pub fn connection_id_valid(
        &mut self,
        source_addr: CanonicalSocketAddr,
        connection_id: ConnectionId,
    ) -> bool {
        let valid_until = connection_id.0.to_ne_bytes()[..4].try_into().unwrap();

        // Check that recreating ConnectionId with same inputs yields identical hash.
        //
        // I expect i64 comparison to be be constant-time.
        if connection_id != self.create_connection_id_inner(valid_until, source_addr) {
            return false;
        }

        u32::from_ne_bytes(valid_until) > self.start_time.elapsed().as_secs() as u32
    }

    fn create_connection_id_inner(
        &mut self,
        valid_until: [u8; 4],
        source_addr: CanonicalSocketAddr,
    ) -> ConnectionId {
        let mut connection_id_bytes = [0u8; 8];

        (&mut connection_id_bytes[..4]).copy_from_slice(&valid_until);

        self.hmac.update(&valid_until);

        match source_addr.get().ip() {
            IpAddr::V4(ip) => self.hmac.update(&ip.octets()),
            IpAddr::V6(ip) => self.hmac.update(&ip.octets()),
        };

        self.hmac.finalize_xof().fill(&mut connection_id_bytes[4..]);
        self.hmac.reset();

        ConnectionId(i64::from_ne_bytes(connection_id_bytes))
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
    AnnounceIpv4(AnnounceResponse<Ipv4Addr>),
    AnnounceIpv6(AnnounceResponse<Ipv6Addr>),
    Scrape(PendingScrapeResponse),
}

#[derive(Clone, Copy, Debug)]
pub struct SocketWorkerIndex(pub usize);

#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub struct RequestWorkerIndex(pub usize);

impl RequestWorkerIndex {
    pub fn from_info_hash(config: &Config, info_hash: InfoHash) -> Self {
        Self(info_hash.0[0] as usize % config.request_workers)
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

    pub fn try_send_to(
        &self,
        index: RequestWorkerIndex,
        request: ConnectedRequest,
        addr: CanonicalSocketAddr,
    ) {
        match self.senders[index.0].try_send((self.index, request, addr)) {
            Ok(()) => {}
            Err(TrySendError::Full(_)) => {
                ::log::error!("Request channel {} is full, dropping request. Try increasing number of request workers or raising config.worker_channel_size.", index.0)
            }
            Err(TrySendError::Disconnected(_)) => {
                panic!("Request channel {} is disconnected", index.0);
            }
        }
    }
}

pub struct ConnectedResponseSender {
    senders: Vec<Sender<(ConnectedResponse, CanonicalSocketAddr)>>,
}

impl ConnectedResponseSender {
    pub fn new(senders: Vec<Sender<(ConnectedResponse, CanonicalSocketAddr)>>) -> Self {
        Self { senders }
    }

    pub fn try_send_to(
        &self,
        index: SocketWorkerIndex,
        response: ConnectedResponse,
        addr: CanonicalSocketAddr,
    ) {
        match self.senders[index.0].try_send((response, addr)) {
            Ok(()) => {}
            Err(TrySendError::Full(_)) => {
                ::log::error!("Response channel {} is full, dropping response. Try increasing number of socket workers or raising config.worker_channel_size.", index.0)
            }
            Err(TrySendError::Disconnected(_)) => {
                panic!("Response channel {} is disconnected", index.0);
            }
        }
    }
}

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
        } else if bytes_left.0 == 0 {
            Self::Seeding
        } else {
            Self::Leeching
        }
    }
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
    pub fn new(num_request_workers: usize) -> Self {
        Self {
            requests_received: Default::default(),
            responses_sent_connect: Default::default(),
            responses_sent_announce: Default::default(),
            responses_sent_scrape: Default::default(),
            responses_sent_error: Default::default(),
            bytes_received: Default::default(),
            bytes_sent: Default::default(),
            torrents: Self::create_atomic_usize_vec(num_request_workers),
            peers: Self::create_atomic_usize_vec(num_request_workers),
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
    pub fn new(num_request_workers: usize) -> Self {
        Self {
            access_list: Arc::new(AccessListArcSwap::default()),
            statistics_ipv4: Arc::new(Statistics::new(num_request_workers)),
            statistics_ipv6: Arc::new(Statistics::new(num_request_workers)),
        }
    }
}

#[cfg(test)]
mod tests {
    use std::net::Ipv6Addr;

    use crate::{common::MAX_PACKET_SIZE, config::Config};

    #[test]
    fn test_peer_status_from_event_and_bytes_left() {
        use crate::common::*;

        use PeerStatus::*;

        let f = PeerStatus::from_event_and_bytes_left;

        assert_eq!(Stopped, f(AnnounceEvent::Stopped, NumberOfBytes(0)));
        assert_eq!(Stopped, f(AnnounceEvent::Stopped, NumberOfBytes(1)));

        assert_eq!(Seeding, f(AnnounceEvent::Started, NumberOfBytes(0)));
        assert_eq!(Leeching, f(AnnounceEvent::Started, NumberOfBytes(1)));

        assert_eq!(Seeding, f(AnnounceEvent::Completed, NumberOfBytes(0)));
        assert_eq!(Leeching, f(AnnounceEvent::Completed, NumberOfBytes(1)));

        assert_eq!(Seeding, f(AnnounceEvent::None, NumberOfBytes(0)));
        assert_eq!(Leeching, f(AnnounceEvent::None, NumberOfBytes(1)));
    }

    // Assumes that announce response with maximum amount of ipv6 peers will
    // be the longest
    #[test]
    fn test_max_package_size() {
        use aquatic_udp_protocol::*;

        let config = Config::default();

        let peers = ::std::iter::repeat(ResponsePeer {
            ip_address: Ipv6Addr::new(1, 1, 1, 1, 1, 1, 1, 1),
            port: Port(1),
        })
        .take(config.protocol.max_response_peers)
        .collect();

        let response = Response::AnnounceIpv6(AnnounceResponse {
            transaction_id: TransactionId(1),
            announce_interval: AnnounceInterval(1),
            seeders: NumberOfPeers(1),
            leechers: NumberOfPeers(1),
            peers,
        });

        let mut buf = Vec::new();

        response.write(&mut buf).unwrap();

        println!("Buffer len: {}", buf.len());

        assert!(buf.len() <= MAX_PACKET_SIZE);
    }
}
