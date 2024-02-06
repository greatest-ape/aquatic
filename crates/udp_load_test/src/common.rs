use std::sync::{atomic::AtomicUsize, Arc};

use aquatic_common::IndexMap;
use aquatic_udp_protocol::*;

#[derive(Clone)]
pub struct LoadTestState {
    pub info_hashes: Arc<[InfoHash]>,
    pub statistics: Arc<SharedStatistics>,
}

#[derive(Default)]
pub struct SharedStatistics {
    pub requests: AtomicUsize,
    pub response_peers: AtomicUsize,
    pub responses_connect: AtomicUsize,
    pub responses_announce: AtomicUsize,
    pub responses_scrape: AtomicUsize,
    pub responses_error: AtomicUsize,
}

pub struct Peer {
    pub announce_info_hash_index: usize,
    pub announce_info_hash: InfoHash,
    pub announce_port: Port,
    pub scrape_info_hash_indices: Box<[usize]>,
    pub socket_index: u8,
}

pub enum StatisticsMessage {
    ResponsesPerInfoHash(IndexMap<usize, u64>),
}
