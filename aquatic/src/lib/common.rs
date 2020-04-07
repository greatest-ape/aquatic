use std::sync::Arc;
use std::sync::atomic::AtomicUsize;
use std::net::{SocketAddr, IpAddr};
use std::time::Instant;

use dashmap::DashMap;
use indexmap::IndexMap;

pub use bittorrent_udp::types::*;


pub const MAX_PACKET_SIZE: usize = 4096;


#[derive(Debug, Clone, Copy)]
pub struct Time(pub Instant);


#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct ConnectionKey {
    pub connection_id: ConnectionId,
    pub socket_addr: SocketAddr
}

pub type ConnectionMap = DashMap<ConnectionKey, Time>;


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
    pub fn from_event_and_bytes_left(
        event:      AnnounceEvent,
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
    pub id: PeerId,
    pub connection_id: ConnectionId,
    pub ip_address: IpAddr,
    pub port: Port,
    pub status: PeerStatus,
    pub last_announce: Time
}


impl Peer {
    pub fn to_response_peer(&self) -> ResponsePeer {
        ResponsePeer {
            ip_address: self.ip_address,
            port: self.port
        }
    }
    pub fn from_announce_and_ip(
        announce_request: &AnnounceRequest,
        ip_address: IpAddr
    ) -> Self {
        Self {
            id: announce_request.peer_id,
            connection_id: announce_request.connection_id,
            ip_address,
            port: announce_request.port,
            status: PeerStatus::from_event_and_bytes_left(
                announce_request.event,
                announce_request.bytes_left
            ),
            last_announce: Time(Instant::now())
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
    pub num_seeders: AtomicUsize,
    pub num_leechers: AtomicUsize,
}


impl Default for TorrentData {
    fn default() -> Self {
        Self {
            peers: IndexMap::new(),
            num_seeders: AtomicUsize::new(0),
            num_leechers: AtomicUsize::new(0),
        }
    }
}


pub type TorrentMap = DashMap<InfoHash, TorrentData>;


#[derive(Default)]
pub struct Statistics {
    pub requests_received: AtomicUsize,
    pub responses_sent: AtomicUsize,
    pub readable_events: AtomicUsize,
}




#[derive(Clone)]
pub struct State {
    pub connections: Arc<ConnectionMap>,
    pub torrents: Arc<TorrentMap>,
    pub statistics: Arc<Statistics>,
}

impl State {
    pub fn new() -> Self {
        Self {
            connections: Arc::new(DashMap::new()),
            torrents: Arc::new(DashMap::new()),
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