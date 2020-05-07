use std::net::SocketAddr;
use std::time::Instant;
use std::sync::Arc;

use flume::{Sender, Receiver};
use hashbrown::HashMap;
use indexmap::IndexMap;
use parking_lot::Mutex;

use crate::protocol::*;


pub struct ValidUntil(pub Instant);


pub struct Peer {
    pub peer_id: PeerId, // FIXME: maybe this field can be removed
    pub complete: bool,
    pub valid_until: ValidUntil,
    pub connection_meta: ConnectionMeta,
}


pub type PeerMap = IndexMap<PeerId, Peer>;


pub struct TorrentData {
    pub peers: PeerMap,
    pub seeders: usize,
    pub leechers: usize,
}


impl Default for TorrentData {
    fn default() -> Self {
        Self {
            peers: IndexMap::new(),
            seeders: 0,
            leechers: 0,
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


#[derive(Clone, Copy)]
pub struct ConnectionMeta {
    /// Index of socket worker responsible for this connection. Required for
    /// sending back response through correct channel to correct worker.
    pub socket_worker_index: usize,
    /// SocketAddr of peer
    pub peer_socket_addr: SocketAddr,
    /// Slab index of PeerConnection
    pub socket_worker_slab_index: usize,
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
        self.0[meta.socket_worker_index].send((meta, message));
    }
}