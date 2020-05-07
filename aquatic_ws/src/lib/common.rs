use std::net::SocketAddr;
use std::time::Instant;

use flume::{Sender, Receiver};
use hashbrown::HashMap;
use indexmap::IndexMap;

use crate::protocol::*;


pub struct ValidUntil(pub Instant);


pub struct Peer {
    pub peer_id: PeerId,
    pub complete: bool,
    pub valid_until: ValidUntil,

    // FIXME: these three could probably be replaced with MessageMeta
    pub socket_worker_index: usize,
    pub socket_addr: SocketAddr,
    pub connection_index: usize,
}


pub type PeerMap = IndexMap<PeerId, Peer>;


pub struct TorrentData {
    pub peers: PeerMap,
    pub seeders: usize,
    pub leechers: usize,
}


pub type TorrentMap = HashMap<InfoHash, TorrentData>;


pub struct State {
    pub torrents: TorrentMap,
}


impl Default for State {
    fn default() -> Self {
        Self {
            torrents: HashMap::new(),
        }
    }
}


pub struct MessageMeta {
    /// Index of socket worker that read this request. Required for sending
    /// back response through correct channel to correct worker.
    pub socket_worker_index: usize,
    /// SocketAddr of peer
    pub peer_socket_addr: SocketAddr,
    /// Slab index of PeerConnection
    pub peer_connection_index: usize,
}


pub type InMessageSender = Sender<(MessageMeta, InMessage)>;
pub type InMessageReceiver = Receiver<(MessageMeta, InMessage)>;
pub type OutMessageReceiver = Receiver<(MessageMeta, OutMessage)>;


pub struct OutMessageSender(Vec<Sender<(MessageMeta, OutMessage)>>);


impl OutMessageSender {
    pub fn new(senders: Vec<Sender<(MessageMeta, OutMessage)>>) -> Self {
        Self(senders)
    }
    pub fn send(
        &self,
        meta: MessageMeta,
        message: OutMessage
    ){
        self.0[meta.socket_worker_index].send((meta, message));
    }
}