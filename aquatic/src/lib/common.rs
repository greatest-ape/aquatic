use std::net::{SocketAddr, IpAddr};
use std::sync::{Arc, atomic::AtomicUsize};
use std::time::{Duration, Instant};

use hashbrown::HashMap;
use indexmap::IndexMap;
use parking_lot::Mutex;

pub use bittorrent_udp::types::*;


pub const MAX_PACKET_SIZE: usize = 4096;


/// Peer or connection valid until this instant
/// 
/// Used instead of "last seen" or similar to hopefully prevent arithmetic
/// overflow when cleaning.
#[derive(Debug, Clone, Copy)]
pub struct ValidUntil(pub Instant);


impl ValidUntil {
    pub fn new(offset_seconds: u64) -> Self {
        Self(Instant::now() + Duration::from_secs(offset_seconds))
    }
}


#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct ConnectionKey {
    pub connection_id: ConnectionId,
    pub socket_addr: SocketAddr
}


pub type ConnectionMap = HashMap<ConnectionKey, ValidUntil>;


#[derive(PartialEq, Eq, Hash, Clone, Copy, Debug)]
pub enum PeerStatus {
    Seeding,
    Leeching,
    Stopped
}


impl PeerStatus {
    /// Determine peer status from announce event and number of bytes left.
    /// 
    /// Likely, the last branch will be taken most of the time.
    #[inline]
    pub fn from_event_and_bytes_left(
        event: AnnounceEvent,
        bytes_left: NumberOfBytes
    ) -> Self {
        if event == AnnounceEvent::Stopped {
            Self::Stopped
        } else if bytes_left.0 == 0 {
            Self::Seeding
        } else {
            Self::Leeching
        }
    }
}


#[derive(Clone, Debug)]
pub struct Peer {
    pub ip_address: IpAddr,
    pub port: Port,
    pub status: PeerStatus,
    pub valid_until: ValidUntil
}


impl Peer {
    #[inline(always)]
    pub fn to_response_peer(&self) -> ResponsePeer {
        ResponsePeer {
            ip_address: self.ip_address,
            port: self.port
        }
    }
}


#[derive(PartialEq, Eq, Hash, Clone)]
pub struct PeerMapKey {
    pub ip: IpAddr,
    pub peer_id: PeerId
}


pub type PeerMap = IndexMap<PeerMapKey, Peer>;


pub struct TorrentData {
    pub peers: PeerMap,
    pub num_seeders: usize,
    pub num_leechers: usize,
}


impl Default for TorrentData {
    fn default() -> Self {
        Self {
            peers: IndexMap::new(),
            num_seeders: 0,
            num_leechers: 0,
        }
    }
}


pub type TorrentMap = HashMap<InfoHash, TorrentData>;


#[derive(Default)]
pub struct Statistics {
    pub requests_received: AtomicUsize,
    pub responses_sent: AtomicUsize,
    pub readable_events: AtomicUsize,
    pub bytes_received: AtomicUsize,
    pub bytes_sent: AtomicUsize,
}


#[derive(Clone)]
pub struct State {
    pub connections: Arc<Mutex<ConnectionMap>>,
    pub torrents: Arc<Mutex<TorrentMap>>,
    pub statistics: Arc<Statistics>,
}


impl State {
    pub fn new() -> Self {
        Self {
            connections: Arc::new(Mutex::new(HashMap::new())),
            torrents: Arc::new(Mutex::new(HashMap::new())),
            statistics: Arc::new(Statistics::default()),
        }
    }
}


#[cfg(test)]
mod tests {
    #[test]
    fn test_peer_status_from_event_and_bytes_left(){
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
}