use std::net::{Ipv4Addr, Ipv6Addr, SocketAddr};
use std::sync::Arc;

use either::Either;
use crossbeam_channel::{Sender, Receiver};
use hashbrown::HashMap;
use indexmap::IndexMap;
use log::error;
use mio::Token;
use parking_lot::Mutex;
use smartstring::{SmartString, LazyCompact};

pub use aquatic_common::{ValidUntil, convert_ipv4_mapped_ipv6};

use aquatic_http_protocol::common::*;
use aquatic_http_protocol::request::Request;
use aquatic_http_protocol::response::{Response, ResponsePeer};


pub const LISTENER_TOKEN: Token = Token(0);
pub const CHANNEL_TOKEN: Token = Token(1);


pub trait Ip: Copy + Eq + ::std::hash::Hash {}

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
    Stopped
}


impl PeerStatus {
    /// Determine peer status from announce event and number of bytes left.
    /// 
    /// Likely, the last branch will be taken most of the time.
    #[inline]
    pub fn from_event_and_bytes_left(
        event: AnnounceEvent,
        opt_bytes_left: Option<usize>
    ) -> Self {
        if let AnnounceEvent::Stopped = event {
            Self::Stopped
        } else if let Some(0) = opt_bytes_left {
            Self::Seeding
        } else {
            Self::Leeching
        }
    }
}


#[derive(Clone, Copy)]
pub struct Peer<I: Ip> {
    pub connection_meta: PeerConnectionMeta<I>,
    pub port: u16,
    pub status: PeerStatus,
    pub valid_until: ValidUntil,
}


impl <I: Ip>Peer<I> {
    pub fn to_response_peer(&self) -> ResponsePeer<I> {
        ResponsePeer {
            ip_address: self.connection_meta.peer_ip_address,
            port: self.port
        }
    }
}


#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct PeerMapKey<I: Ip> {
    pub peer_id: PeerId,
    pub ip_or_key: Either<I, SmartString<LazyCompact>>
}


pub type PeerMap<I> = IndexMap<PeerMapKey<I>, Peer<I>>;


pub struct TorrentData<I: Ip> {
    pub peers: PeerMap<I>,
    pub num_seeders: usize,
    pub num_leechers: usize,
}


impl <I: Ip> Default for TorrentData<I> {
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


#[derive(Clone)]
pub struct State {
    pub torrent_maps: Arc<Mutex<TorrentMaps>>,
}


impl Default for State {
    fn default() -> Self {
        Self {
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
    pub fn new(
        senders: Vec<Sender<(ConnectionMeta, Response)>>,
    ) -> Self {
        Self {
            senders,
        }
    }

    #[inline]
    pub fn send(
        &self,
        meta: ConnectionMeta,
        message: Response
    ){
        if let Err(err) = self.senders[meta.worker_index].send((meta, message)){
            error!("ResponseChannelSender: couldn't send message: {:?}", err);
        }
    }
}


pub type SocketWorkerStatus = Option<Result<(), String>>;
pub type SocketWorkerStatuses = Arc<Mutex<Vec<SocketWorkerStatus>>>;