use std::net::{Ipv4Addr, Ipv6Addr, SocketAddr};
use std::time::Instant;

use aquatic_common::access_list::AccessList;
use either::Either;
use hashbrown::HashMap;
use indexmap::IndexMap;
use smartstring::{LazyCompact, SmartString};

pub use aquatic_common::{convert_ipv4_mapped_ipv6, ValidUntil};

use aquatic_http_protocol::common::*;
use aquatic_http_protocol::response::ResponsePeer;

use crate::config::Config;

use std::borrow::Borrow;
use std::cell::RefCell;
use std::rc::Rc;

use futures_lite::AsyncBufReadExt;
use glommio::io::{BufferedFile, StreamReaderBuilder};
use glommio::prelude::*;

use aquatic_http_protocol::{
    request::{AnnounceRequest, ScrapeRequest},
    response::{AnnounceResponse, ScrapeResponse},
};


#[derive(Copy, Clone, Debug)]
pub struct ConsumerId(pub usize);

#[derive(Clone, Copy, Debug)]
pub struct ConnectionId(pub usize);

#[derive(Debug)]
pub enum ChannelRequest {
    Announce {
        request: AnnounceRequest,
        peer_addr: SocketAddr,
        connection_id: ConnectionId,
        response_consumer_id: ConsumerId,
    },
    Scrape {
        request: ScrapeRequest,
        peer_addr: SocketAddr,
        connection_id: ConnectionId,
        response_consumer_id: ConsumerId,
    },
}

#[derive(Debug)]
pub enum ChannelResponse {
    Announce {
        response: AnnounceResponse,
        peer_addr: SocketAddr,
        connection_id: ConnectionId,
    },
    Scrape {
        response: ScrapeResponse,
        peer_addr: SocketAddr,
        connection_id: ConnectionId,
    },
}

impl ChannelResponse {
    pub fn get_connection_id(&self) -> ConnectionId {
        match self {
            Self::Announce { connection_id, .. } => *connection_id,
            Self::Scrape { connection_id, .. } => *connection_id,
        }
    }
    pub fn get_peer_addr(&self) -> SocketAddr {
        match self {
            Self::Announce { peer_addr, .. } => *peer_addr,
            Self::Scrape { peer_addr, .. } => *peer_addr,
        }
    }
}

pub async fn update_access_list<C: Borrow<Config>>(
    config: C,
    access_list: Rc<RefCell<AccessList>>,
) {
    if config.borrow().access_list.mode.is_on() {
        match BufferedFile::open(&config.borrow().access_list.path).await {
            Ok(file) => {
                let mut reader = StreamReaderBuilder::new(file).build();
                let mut new_access_list = AccessList::default();

                loop {
                    let mut buf = String::with_capacity(42);

                    match reader.read_line(&mut buf).await {
                        Ok(_) => {
                            if let Err(err) = new_access_list.insert_from_line(&buf) {
                                ::log::error!(
                                    "Couln't parse access list line '{}': {:?}",
                                    buf,
                                    err
                                );
                            }
                        }
                        Err(err) => {
                            ::log::error!("Couln't read access list line {:?}", err);

                            break;
                        }
                    }

                    yield_if_needed().await;
                }

                *access_list.borrow_mut() = new_access_list;
            }
            Err(err) => {
                ::log::error!("Couldn't open access list file: {:?}", err)
            }
        };
    }
}

pub trait Ip: ::std::fmt::Debug + Copy + Eq + ::std::hash::Hash {}

impl Ip for Ipv4Addr {}
impl Ip for Ipv6Addr {}

#[derive(Clone, Copy, Debug)]
pub struct ConnectionMeta {
    /// Index of socket worker responsible for this connection. Required for
    /// sending back response through correct channel to correct worker.
    pub worker_index: usize, // Or response consumer id in glommio
    pub peer_addr: SocketAddr,
    pub poll_token: usize, // Or connection id in glommio
}

#[derive(Clone, Copy, Debug)]
pub struct PeerConnectionMeta<I: Ip> {
    pub worker_index: usize,
    pub poll_token: usize,
    pub peer_ip_address: I,
}

#[derive(PartialEq, Eq, Clone, Copy, Debug)]
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
    pub fn from_event_and_bytes_left(event: AnnounceEvent, opt_bytes_left: Option<usize>) -> Self {
        if let AnnounceEvent::Stopped = event {
            Self::Stopped
        } else if let Some(0) = opt_bytes_left {
            Self::Seeding
        } else {
            Self::Leeching
        }
    }
}

#[derive(Debug, Clone, Copy)]
pub struct Peer<I: Ip> {
    pub connection_meta: PeerConnectionMeta<I>,
    pub port: u16,
    pub status: PeerStatus,
    pub valid_until: ValidUntil,
}

impl<I: Ip> Peer<I> {
    pub fn to_response_peer(&self) -> ResponsePeer<I> {
        ResponsePeer {
            ip_address: self.connection_meta.peer_ip_address,
            port: self.port,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct PeerMapKey<I: Ip> {
    pub peer_id: PeerId,
    pub ip_or_key: Either<I, SmartString<LazyCompact>>,
}

pub type PeerMap<I> = IndexMap<PeerMapKey<I>, Peer<I>>;

pub struct TorrentData<I: Ip> {
    pub peers: PeerMap<I>,
    pub num_seeders: usize,
    pub num_leechers: usize,
}

impl<I: Ip> Default for TorrentData<I> {
    #[inline]
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
    pub fn clean(&mut self, config: &Config, access_list: &AccessList) {
        Self::clean_torrent_map(config, access_list, &mut self.ipv4);
        Self::clean_torrent_map(config, access_list, &mut self.ipv6);
    }

    fn clean_torrent_map<I: Ip>(
        config: &Config,
        access_list: &AccessList,
        torrent_map: &mut TorrentMap<I>,
    ) {
        let now = Instant::now();

        torrent_map.retain(|info_hash, torrent_data| {
            if !access_list.allows(config.access_list.mode, &info_hash.0) {
                return false;
            }

            let num_seeders = &mut torrent_data.num_seeders;
            let num_leechers = &mut torrent_data.num_leechers;

            torrent_data.peers.retain(|_, peer| {
                let keep = peer.valid_until.0 >= now;

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

            !torrent_data.peers.is_empty()
        });

        torrent_map.shrink_to_fit();
    }
}

pub fn num_digits_in_usize(mut number: usize) -> usize {
    let mut num_digits = 1usize;

    while number >= 10 {
        num_digits += 1;

        number /= 10;
    }

    num_digits
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_num_digits_in_usize() {
        let f = num_digits_in_usize;

        assert_eq!(f(0), 1);
        assert_eq!(f(1), 1);
        assert_eq!(f(9), 1);
        assert_eq!(f(10), 2);
        assert_eq!(f(11), 2);
        assert_eq!(f(99), 2);
        assert_eq!(f(100), 3);
        assert_eq!(f(101), 3);
        assert_eq!(f(1000), 4);
    }
}