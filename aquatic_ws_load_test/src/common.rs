use std::sync::{Arc, atomic::AtomicUsize};

use rand_distr::Pareto;

pub use aquatic_ws_protocol::*;


/// JSON string used as fake announce offer / offer response
pub const JSON_VALUE: &str = r#"{ "offer": "abcdefghabcdefghabcdefghabcdefghabcdefghabcdefghabcdefghabcdefghabcdefghabcdefghabcdefghabcdefghabcdefghabcdefghabcdefgh" }"#;


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
    pub responses_offer: AtomicUsize,
    pub responses_answer: AtomicUsize,
    pub responses_scrape: AtomicUsize,
}


#[derive(Clone)]
pub struct LoadTestState {
    pub info_hashes: Arc<Vec<InfoHash>>,
    pub statistics: Arc<Statistics>,
    pub pareto: Arc<Pareto<f64>>,
}


#[derive(PartialEq, Eq, Clone, Copy)]
pub enum RequestType {
    Announce,
    Scrape
}