use std::sync::{atomic::AtomicUsize, Arc};

use aquatic_ws_protocol::common::InfoHash;
use rand_distr::Gamma;

#[derive(Default)]
pub struct Statistics {
    pub requests: AtomicUsize,
    pub response_peers: AtomicUsize,
    pub responses_announce: AtomicUsize,
    pub responses_offer: AtomicUsize,
    pub responses_answer: AtomicUsize,
    pub responses_scrape: AtomicUsize,
    pub responses_error: AtomicUsize,
    pub connections: AtomicUsize,
}

#[derive(Clone)]
pub struct LoadTestState {
    pub info_hashes: Arc<[InfoHash]>,
    pub statistics: Arc<Statistics>,
    pub gamma: Arc<Gamma<f64>>,
}

#[derive(PartialEq, Eq, Clone, Copy)]
pub enum RequestType {
    Announce,
    Scrape,
}
