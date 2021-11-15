use std::hash::Hash;
use std::net::{IpAddr, Ipv4Addr, Ipv6Addr};
use std::sync::atomic::AtomicUsize;
use std::sync::Arc;
use std::time::Instant;

use parking_lot::Mutex;
use socket2::{Domain, Protocol, Socket, Type};

use aquatic_common::access_list::{create_access_list_cache, AccessListArcSwap};
use aquatic_common::AHashIndexMap;
use aquatic_common::ValidUntil;
use aquatic_udp_protocol::*;

use crate::config::Config;

pub mod network;

pub const MAX_PACKET_SIZE: usize = 8192;

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

#[derive(Debug)]
pub enum ConnectedRequest {
    Announce(AnnounceRequest),
    Scrape {
        request: ScrapeRequest,
        // FIXME
        original_indices: Vec<usize>,
    },
}

#[derive(Debug)]
pub enum ConnectedResponse {
    AnnounceIpv4(AnnounceResponseIpv4),
    AnnounceIpv6(AnnounceResponseIpv6),
    Scrape {
        response: ScrapeResponse,
        /// Currently only used by glommio implementation
        original_indices: Vec<usize>,
    },
}

impl Into<Response> for ConnectedResponse {
    fn into(self) -> Response {
        match self {
            Self::AnnounceIpv4(response) => Response::AnnounceIpv4(response),
            Self::AnnounceIpv6(response) => Response::AnnounceIpv6(response),
            Self::Scrape { response, .. } => Response::Scrape(response),
        }
    }
}

#[derive(Clone, PartialEq, Debug)]
pub struct ProtocolResponsePeer<I> {
    pub ip_address: I,
    pub port: Port,
}

pub struct ProtocolAnnounceResponse<I> {
    pub transaction_id: TransactionId,
    pub announce_interval: AnnounceInterval,
    pub leechers: NumberOfPeers,
    pub seeders: NumberOfPeers,
    pub peers: Vec<ProtocolResponsePeer<I>>,
}

impl Into<ConnectedResponse> for ProtocolAnnounceResponse<Ipv4Addr> {
    fn into(self) -> ConnectedResponse {
        ConnectedResponse::AnnounceIpv4(AnnounceResponseIpv4 {
            fixed: AnnounceResponseIpv4Fixed {
                action: AnnounceAction::new(),
                transaction_id: self.transaction_id,
                announce_interval: self.announce_interval,
                leechers: self.leechers,
                seeders: self.seeders,
            },
            peers: self
                .peers
                .into_iter()
                .map(|peer| ResponsePeerIpv4 {
                    ip_address: peer.ip_address.octets(),
                    port: peer.port,
                })
                .collect(),
        })
    }
}

impl Into<ConnectedResponse> for ProtocolAnnounceResponse<Ipv6Addr> {
    fn into(self) -> ConnectedResponse {
        ConnectedResponse::AnnounceIpv6(AnnounceResponseIpv6 {
            fixed: AnnounceResponseIpv6Fixed {
                action: AnnounceIpv6Action::new(),
                transaction_id: self.transaction_id,
                announce_interval: self.announce_interval,
                leechers: self.leechers,
                seeders: self.seeders,
            },
            peers: self
                .peers
                .into_iter()
                .map(|peer| ResponsePeerIpv6 {
                    ip_address: peer.ip_address.octets(),
                    port: peer.port,
                })
                .collect(),
        })
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
        } else if bytes_left.0.get() == 0 {
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
    pub fn to_response_peer(&self) -> ProtocolResponsePeer<I> {
        ProtocolResponsePeer {
            ip_address: self.ip_address,
            port: self.port,
        }
    }
}

#[derive(PartialEq, Eq, Hash, Clone, Copy)]
pub struct PeerMapKey<I: Ip> {
    pub ip: I,
    pub peer_id: PeerId,
}

pub type PeerMap<I> = AHashIndexMap<PeerMapKey<I>, Peer<I>>;

pub struct TorrentData<I: Ip> {
    pub peers: PeerMap<I>,
    pub num_seeders: usize,
    pub num_leechers: usize,
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

pub type TorrentMap<I> = AHashIndexMap<InfoHash, TorrentData<I>>;

#[derive(Default)]
pub struct TorrentMaps {
    pub ipv4: TorrentMap<Ipv4Addr>,
    pub ipv6: TorrentMap<Ipv6Addr>,
}

impl TorrentMaps {
    /// Remove disallowed and inactive torrents
    pub fn clean(&mut self, config: &Config, access_list: &Arc<AccessListArcSwap>) {
        let now = Instant::now();
        let access_list_mode = config.access_list.mode;

        let mut access_list_cache = create_access_list_cache(access_list);

        self.ipv4.retain(|info_hash, torrent| {
            access_list_cache
                .load()
                .allows(access_list_mode, &info_hash.0)
                && Self::clean_torrent_and_peers(now, torrent)
        });
        self.ipv4.shrink_to_fit();

        self.ipv6.retain(|info_hash, torrent| {
            access_list_cache
                .load()
                .allows(access_list_mode, &info_hash.0)
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

        torrent.peers.shrink_to_fit();

        !torrent.peers.is_empty()
    }
}

#[derive(Default)]
pub struct Statistics {
    pub requests_received: AtomicUsize,
    pub responses_sent: AtomicUsize,
    pub bytes_received: AtomicUsize,
    pub bytes_sent: AtomicUsize,
}

#[derive(Clone)]
pub struct State {
    pub access_list: Arc<AccessListArcSwap>,
    pub torrents: Arc<Mutex<TorrentMaps>>,
    pub statistics: Arc<Statistics>,
}

impl Default for State {
    fn default() -> Self {
        Self {
            access_list: Arc::new(AccessListArcSwap::default()),
            torrents: Arc::new(Mutex::new(TorrentMaps::default())),
            statistics: Arc::new(Statistics::default()),
        }
    }
}
pub fn ip_version_from_ip(ip: IpAddr) -> IpVersion {
    match ip {
        IpAddr::V4(_) => IpVersion::IPv4,
        IpAddr::V6(ip) => {
            if let [0, 0, 0, 0, 0, 0xffff, ..] = ip.segments() {
                IpVersion::IPv4
            } else {
                IpVersion::IPv6
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use crate::{common::MAX_PACKET_SIZE, config::Config};

    #[test]
    fn test_peer_status_from_event_and_bytes_left() {
        use crate::common::*;

        use PeerStatus::*;

        let f = PeerStatus::from_event_and_bytes_left;

        assert_eq!(Stopped, f(AnnounceEvent::Stopped, NumberOfBytes(0.into())));
        assert_eq!(Stopped, f(AnnounceEvent::Stopped, NumberOfBytes(1.into())));

        assert_eq!(Seeding, f(AnnounceEvent::Started, NumberOfBytes(0.into())));
        assert_eq!(Leeching, f(AnnounceEvent::Started, NumberOfBytes(1.into())));

        assert_eq!(
            Seeding,
            f(AnnounceEvent::Completed, NumberOfBytes(0.into()))
        );
        assert_eq!(
            Leeching,
            f(AnnounceEvent::Completed, NumberOfBytes(1.into()))
        );

        assert_eq!(Seeding, f(AnnounceEvent::None, NumberOfBytes(0.into())));
        assert_eq!(Leeching, f(AnnounceEvent::None, NumberOfBytes(1.into())));
    }

    // Assumes that announce response with maximum amount of ipv6 peers will
    // be the longest
    #[test]
    fn test_max_package_size() {
        use aquatic_udp_protocol::*;

        let config = Config::default();

        let peers = ::std::iter::repeat(ResponsePeerIpv6 {
            ip_address: [1; 16],
            port: Port(1.into()),
        })
        .take(config.protocol.max_response_peers)
        .collect();

        let response = Response::AnnounceIpv6(AnnounceResponseIpv6 {
            fixed: AnnounceResponseIpv6Fixed {
                action: AnnounceIpv6Action::new(),
                transaction_id: TransactionId(1.into()),
                announce_interval: AnnounceInterval(1.into()),
                seeders: NumberOfPeers(1.into()),
                leechers: NumberOfPeers(1.into()),
            },
            peers,
        });

        let mut buf = Vec::new();

        response.write(&mut buf).unwrap();

        println!("Buffer len: {}", buf.len());

        assert!(buf.len() <= MAX_PACKET_SIZE);
    }
}
