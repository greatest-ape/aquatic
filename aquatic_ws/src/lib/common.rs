use std::net::SocketAddr;
use std::sync::Arc;

use flume::{Sender, Receiver};
use hashbrown::HashMap;
use indexmap::IndexMap;
use parking_lot::Mutex;
use mio::Token;

pub use aquatic_udp::common::ValidUntil;

use crate::protocol::*;


#[derive(Clone, Copy)]
pub struct ConnectionMeta {
    /// Index of socket worker responsible for this connection. Required for
    /// sending back response through correct channel to correct worker.
    pub worker_index: usize,
    pub peer_addr: SocketAddr,
    pub poll_token: Token,
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
        bytes_left: usize
    ) -> Self {
        if let AnnounceEvent::Stopped = event {
            Self::Stopped
        } else if bytes_left == 0 {
            Self::Seeding
        } else {
            Self::Leeching
        }
    }
}


#[derive(Clone, Copy)]
pub struct Peer {
    pub connection_meta: ConnectionMeta,
    pub status: PeerStatus,
    pub valid_until: ValidUntil,
}


pub type PeerMap = IndexMap<PeerId, Peer>;


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


pub struct State {
    pub torrents: Arc<Mutex<TorrentMap>>,
}


impl Default for State {
    fn default() -> Self {
        Self {
            torrents: Arc::new(Mutex::new(HashMap::new())),
        }
    }
}


pub type InMessageSender = Sender<(ConnectionMeta, InMessage)>;
pub type InMessageReceiver = Receiver<(ConnectionMeta, InMessage)>;
pub type OutMessageReceiver = Receiver<(ConnectionMeta, OutMessage)>;


pub struct OutMessageSender(Vec<Sender<(ConnectionMeta, OutMessage)>>);


impl OutMessageSender {
    pub fn new(senders: Vec<Sender<(ConnectionMeta, OutMessage)>>) -> Self {
        Self(senders)
    }
    pub fn send(
        &self,
        meta: ConnectionMeta,
        message: OutMessage
    ){
        self.0[meta.worker_index].send((meta, message));
    }
}