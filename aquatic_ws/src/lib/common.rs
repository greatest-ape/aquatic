use std::net::SocketAddr;
use std::sync::Arc;

use flume::{Sender, Receiver};
use hashbrown::HashMap;
use indexmap::IndexMap;
use mio::Token;
use parking_lot::Mutex;

pub use aquatic_common::ValidUntil;

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
    #[inline]
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
pub struct TorrentMaps {
    pub ipv4: TorrentMap,
    pub ipv6: TorrentMap,
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


pub type InMessageSender = Sender<(ConnectionMeta, InMessage)>;
pub type InMessageReceiver = Receiver<(ConnectionMeta, InMessage)>;
pub type OutMessageReceiver = Receiver<(ConnectionMeta, OutMessage)>;


pub struct OutMessageSender(Vec<Sender<(ConnectionMeta, OutMessage)>>);


impl OutMessageSender {
    pub fn new(senders: Vec<Sender<(ConnectionMeta, OutMessage)>>) -> Self {
        Self(senders)
    }

    #[inline]
    pub fn send(
        &self,
        meta: ConnectionMeta,
        message: OutMessage
    ){
        self.0[meta.worker_index].send((meta, message));
    }
}


pub type SocketWorkerStatus = Option<Result<(), String>>;
pub type SocketWorkerStatuses = Arc<Mutex<Vec<SocketWorkerStatus>>>;
