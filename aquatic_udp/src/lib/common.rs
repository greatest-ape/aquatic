use std::hash::Hash;
use std::net::{IpAddr, Ipv4Addr, Ipv6Addr, SocketAddr};
use std::sync::{atomic::AtomicUsize, Arc};
use std::time::Instant;

use hashbrown::HashMap;
use indexmap::IndexMap;
use parking_lot::Mutex;

pub use aquatic_common::{access_list::AccessList, ValidUntil};
pub use aquatic_udp_protocol::*;

use crate::config::Config;

pub const MAX_PACKET_SIZE: usize = 4096;

pub trait Ip: Hash + PartialEq + Eq + Clone + Copy {
    fn ip_addr(self) -> IpAddr;
}

impl Ip for Ipv4Addr {
    fn ip_addr(self) -> IpAddr {
        IpAddr::V4(self)
    }
}

impl Ip for Ipv6Addr {
    fn ip_addr(self) -> IpAddr {
        IpAddr::V6(self)
    }
}

pub enum ConnectedRequest {
    Announce(AnnounceRequest),
    Scrape(ScrapeRequest),
}

pub enum ConnectedResponse {
    Announce(AnnounceResponse),
    Scrape(ScrapeResponse),
}

impl Into<Response> for ConnectedResponse {
    fn into(self) -> Response {
        match self {
            Self::Announce(response) => Response::Announce(response),
            Self::Scrape(response) => Response::Scrape(response),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct ConnectionKey {
    pub connection_id: ConnectionId,
    pub socket_addr: SocketAddr,
}

impl ConnectionKey {
    pub fn new(connection_id: ConnectionId, socket_addr: SocketAddr) -> Self {
        Self {
            connection_id,
            socket_addr,
        }
    }
}

pub type ConnectionMap = HashMap<ConnectionKey, ValidUntil>;

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

#[derive(Clone, Debug)]
pub struct Peer<I: Ip> {
    pub ip_address: I,
    pub port: Port,
    pub status: PeerStatus,
    pub valid_until: ValidUntil,
}

impl<I: Ip> Peer<I> {
    #[inline(always)]
    pub fn to_response_peer(&self) -> ResponsePeer {
        ResponsePeer {
            ip_address: self.ip_address.ip_addr(),
            port: self.port,
        }
    }
}

#[derive(PartialEq, Eq, Hash, Clone, Copy)]
pub struct PeerMapKey<I: Ip> {
    pub ip: I,
    pub peer_id: PeerId,
}

pub type PeerMap<I> = IndexMap<PeerMapKey<I>, Peer<I>>;

pub struct TorrentData<I: Ip> {
    pub peers: PeerMap<I>,
    pub num_seeders: usize,
    pub num_leechers: usize,
}

impl<I: Ip> Default for TorrentData<I> {
    fn default() -> Self {
        Self {
            peers: IndexMap::new(),
            num_seeders: 0,
            num_leechers: 0,
        }
    }
}

pub type TorrentMap<I> = HashMap<InfoHash, TorrentData<I>>;

#[derive(Default)]
pub struct TorrentMaps {
    pub ipv4: TorrentMap<Ipv4Addr>,
    pub ipv6: TorrentMap<Ipv6Addr>,
}

impl TorrentMaps {
    /// Remove disallowed and inactive torrents
    pub fn clean(&mut self, config: &Config, access_list: &Arc<AccessList>) {
        let now = Instant::now();

        let access_list_mode = config.access_list.mode;

        self.ipv4.retain(|info_hash, torrent| {
            access_list.allows(access_list_mode, &info_hash.0)
                && Self::clean_torrent_and_peers(now, torrent)
        });
        self.ipv4.shrink_to_fit();

        self.ipv6.retain(|info_hash, torrent| {
            access_list.allows(access_list_mode, &info_hash.0)
                && Self::clean_torrent_and_peers(now, torrent)
        });
        self.ipv6.shrink_to_fit();
    }

    /// Returns true if torrent is to be kept
    #[inline]
    fn clean_torrent_and_peers<I: Ip>(now: Instant, torrent: &mut TorrentData<I>) -> bool {
        let num_seeders = &mut torrent.num_seeders;
        let num_leechers = &mut torrent.num_leechers;

        torrent.peers.retain(|_, peer| {
            let keep = peer.valid_until.0 > now;

            if !keep {
                match peer.status {
                    PeerStatus::Seeding => {
                        *num_seeders -= 1;
                    }
                    PeerStatus::Leeching => {
                        *num_leechers -= 1;
                    }
                    _ => (),
                };
            }

            keep
        });

        !torrent.peers.is_empty()
    }
}

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
    pub access_list: Arc<AccessList>,
    pub torrents: Arc<Mutex<TorrentMaps>>,
    pub statistics: Arc<Statistics>,
}

impl Default for State {
    fn default() -> Self {
        Self {
            access_list: Arc::new(AccessList::default()),
            torrents: Arc::new(Mutex::new(TorrentMaps::default())),
            statistics: Arc::new(Statistics::default()),
        }
    }
}

#[cfg(test)]
mod tests {
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
}
