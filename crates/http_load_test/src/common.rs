use std::sync::{atomic::AtomicUsize, Arc};

use rand_distr::Gamma;

pub use aquatic_http_protocol::common::*;
pub use aquatic_http_protocol::request::*;

#[derive(PartialEq, Eq, Clone)]
pub struct TorrentPeer {
    pub info_hash: InfoHash,
    pub scrape_hash_indeces: Vec<usize>,
    pub peer_id: PeerId,
    pub port: u16,
}

#[derive(Default)]
pub struct Statistics {
    pub requests: AtomicUsize,
    pub response_peers: AtomicUsize,
    pub responses_announce: AtomicUsize,
    pub responses_scrape: AtomicUsize,
    pub responses_failure: AtomicUsize,
    pub bytes_sent: AtomicUsize,
    pub bytes_received: AtomicUsize,
}

#[derive(Clone)]
pub struct LoadTestState {
    pub info_hashes: Arc<Vec<InfoHash>>,
    pub statistics: Arc<Statistics>,
    pub gamma: Arc<Gamma<f64>>,
}

#[derive(PartialEq, Eq, Clone, Copy)]
pub enum RequestType {
    Announce,
    Scrape,
}
