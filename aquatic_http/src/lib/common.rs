use std::net::{Ipv4Addr, Ipv6Addr, SocketAddr};
use std::sync::Arc;
use std::time::Instant;

use aquatic_common::access_list::{AccessList, AccessListArcSwap};
use crossbeam_channel::{Receiver, Sender};
use either::Either;
use hashbrown::HashMap;
use indexmap::IndexMap;
use log::error;
use mio::Token;
use parking_lot::Mutex;
use smartstring::{LazyCompact, SmartString};

pub use aquatic_common::{convert_ipv4_mapped_ipv6, ValidUntil};

use aquatic_http_protocol::common::*;
use aquatic_http_protocol::request::Request;
use aquatic_http_protocol::response::{Response, ResponsePeer};

use crate::config::Config;

pub const LISTENER_TOKEN: Token = Token(0);
pub const CHANNEL_TOKEN: Token = Token(1);

pub trait Ip: ::std::fmt::Debug + Copy + Eq + ::std::hash::Hash {}

impl Ip for Ipv4Addr {}
impl Ip for Ipv6Addr {}

#[derive(Clone, Copy, Debug)]
pub struct ConnectionMeta {
    /// Index of socket worker responsible for this connection. Required for
    /// sending back response through correct channel to correct worker.
    pub worker_index: usize,
    pub peer_addr: SocketAddr,
    pub poll_token: Token,
}

#[derive(Clone, Copy, Debug)]
pub struct PeerConnectionMeta<I: Ip> {
    pub worker_index: usize,
    pub poll_token: Token,
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
    pub fn clean(&mut self, config: &Config, access_list: &Arc<AccessList>) {
        Self::clean_torrent_map(config, access_list, &mut self.ipv4);
        Self::clean_torrent_map(config, access_list, &mut self.ipv6);
    }

    fn clean_torrent_map<I: Ip>(
        config: &Config,
        access_list: &Arc<AccessList>,
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

#[derive(Clone)]
pub struct State {
    pub access_list: Arc<AccessListArcSwap>,
    pub torrent_maps: Arc<Mutex<TorrentMaps>>,
}

impl Default for State {
    fn default() -> Self {
        Self {
            access_list: Arc::new(Default::default()),
            torrent_maps: Arc::new(Mutex::new(TorrentMaps::default())),
        }
    }
}

pub type RequestChannelSender = Sender<(ConnectionMeta, Request)>;
pub type RequestChannelReceiver = Receiver<(ConnectionMeta, Request)>;
pub type ResponseChannelReceiver = Receiver<(ConnectionMeta, Response)>;

#[derive(Clone)]
pub struct ResponseChannelSender {
    senders: Vec<Sender<(ConnectionMeta, Response)>>,
}

impl ResponseChannelSender {
    pub fn new(senders: Vec<Sender<(ConnectionMeta, Response)>>) -> Self {
        Self { senders }
    }

    #[inline]
    pub fn send(&self, meta: ConnectionMeta, message: Response) {
        if let Err(err) = self.senders[meta.worker_index].send((meta, message)) {
            error!("ResponseChannelSender: couldn't send message: {:?}", err);
        }
    }
}

pub type SocketWorkerStatus = Option<Result<(), String>>;
pub type SocketWorkerStatuses = Arc<Mutex<Vec<SocketWorkerStatus>>>;
