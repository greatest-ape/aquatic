use std::net::{Ipv4Addr, Ipv6Addr, SocketAddr};
use std::sync::Arc;

use either::Either;
use flume::{Sender, Receiver};
use hashbrown::HashMap;
use indexmap::IndexMap;
use log::error;
use mio::Token;
use parking_lot::Mutex;

pub use aquatic_common::{ValidUntil, convert_ipv4_mapped_ipv4};

use crate::protocol::common::*;
use crate::protocol::request::Request;
use crate::protocol::response::{Response, ResponsePeer};


#[derive(Clone, Copy, Debug)]
pub struct ConnectionMeta {
    /// Index of socket worker responsible for this connection. Required for
    /// sending back response through correct channel to correct worker.
    pub worker_index: usize,
    pub peer_addr: SocketAddr,
    pub poll_token: Token,
}


#[derive(Clone, Copy, Debug)]
pub struct PeerConnectionMeta<P> {
    pub worker_index: usize,
    pub poll_token: Token,
    pub peer_ip_address: P,
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
pub struct Peer<P> {
    pub connection_meta: PeerConnectionMeta<P>,
    pub port: u16,
    pub status: PeerStatus,
    pub valid_until: ValidUntil,
}


impl <S: Copy>Peer<S> {
    pub fn to_response_peer(&self) -> ResponsePeer<S> {
        ResponsePeer {
            ip_address: self.connection_meta.peer_ip_address,
            port: self.port
        }
    }
}


#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct PeerMapKey<P: Eq + ::std::hash::Hash> {
    pub peer_id: PeerId,
    pub ip_or_key: Either<P, String>
}


pub type PeerMap<P> = IndexMap<PeerMapKey<P>, Peer<P>>;


pub struct TorrentData<P: Eq + ::std::hash::Hash> {
    pub peers: PeerMap<P>,
    pub num_seeders: usize,
    pub num_leechers: usize,
}


impl <P: Eq + ::std::hash::Hash> Default for TorrentData<P> {
    #[inline]
    fn default() -> Self {
        Self {
            peers: IndexMap::new(),
            num_seeders: 0,
            num_leechers: 0,
        }
    }
}


pub type TorrentMap<P> = HashMap<InfoHash, TorrentData<P>>;


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


pub struct ResponseChannelSender(Vec<Sender<(ConnectionMeta, Response)>>);


impl ResponseChannelSender {
    pub fn new(senders: Vec<Sender<(ConnectionMeta, Response)>>) -> Self {
        Self(senders)
    }

    #[inline]
    pub fn send(
        &self,
        meta: ConnectionMeta,
        message: Response
    ){
        if let Err(err) = self.0[meta.worker_index].send((meta, message)){
            error!("ResponseChannelSender: couldn't send message: {:?}", err);
        }
    }
}


pub type SocketWorkerStatus = Option<Result<(), String>>;
pub type SocketWorkerStatuses = Arc<Mutex<Vec<SocketWorkerStatus>>>;
