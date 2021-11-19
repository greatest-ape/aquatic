use std::sync::{atomic::AtomicUsize, Arc};

use hashbrown::HashMap;

use aquatic_udp_protocol::*;

#[derive(Debug, PartialEq, Eq, Hash, Clone, Copy)]
pub struct ThreadId(pub u8);

#[derive(PartialEq, Eq, Clone)]
pub struct TorrentPeer {
    pub info_hash: InfoHash,
    pub scrape_hash_indeces: Vec<usize>,
    pub connection_id: ConnectionId,
    pub peer_id: PeerId,
    pub port: Port,
}

pub type TorrentPeerMap = HashMap<TransactionId, TorrentPeer>;

#[derive(Default)]
pub struct Statistics {
    pub requests: AtomicUsize,
    pub response_peers: AtomicUsize,
    pub responses_connect: AtomicUsize,
    pub responses_announce: AtomicUsize,
    pub responses_scrape: AtomicUsize,
    pub responses_error: AtomicUsize,
}

#[derive(Clone)]
pub struct LoadTestState {
    pub info_hashes: Arc<Vec<InfoHash>>,
    pub statistics: Arc<Statistics>,
}

#[derive(PartialEq, Eq, Clone, Copy)]
pub enum RequestType {
    Announce,
    Connect,
    Scrape,
}

#[derive(Default)]
pub struct SocketWorkerLocalStatistics {
    pub requests: usize,
    pub response_peers: usize,
    pub responses_connect: usize,
    pub responses_announce: usize,
    pub responses_scrape: usize,
    pub responses_error: usize,
}
