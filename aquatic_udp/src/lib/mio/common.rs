use aquatic_common::access_list::AccessListArcSwap;
use parking_lot::Mutex;
use std::sync::{atomic::AtomicUsize, Arc};

use crate::common::*;

#[derive(Default)]
pub struct Statistics {
    pub requests_received: AtomicUsize,
    pub responses_sent: AtomicUsize,
    pub bytes_received: AtomicUsize,
    pub bytes_sent: AtomicUsize,
}

#[derive(Clone)]
pub struct State {
    pub access_list: Arc<AccessListArcSwap>,
    pub torrents: Arc<Mutex<TorrentMaps>>,
    pub statistics: Arc<Statistics>,
}

impl Default for State {
    fn default() -> Self {
        Self {
            access_list: Arc::new(AccessListArcSwap::default()),
            torrents: Arc::new(Mutex::new(TorrentMaps::default())),
            statistics: Arc::new(Statistics::default()),
        }
    }
}
