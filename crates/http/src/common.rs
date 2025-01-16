use std::sync::Arc;

use aquatic_common::access_list::AccessListArcSwap;
use aquatic_common::CanonicalSocketAddr;

pub use aquatic_common::ValidUntil;

use aquatic_http_protocol::{
    request::{AnnounceRequest, ScrapeRequest},
    response::{AnnounceResponse, ScrapeResponse},
};
use glommio::channels::shared_channel::SharedSender;
use slotmap::new_key_type;

#[allow(dead_code)]
#[derive(Copy, Clone, Debug)]
pub struct ConsumerId(pub usize);

new_key_type! {
    pub struct ConnectionId;
}

#[derive(Debug)]
pub enum ChannelRequest {
    Announce {
        request: AnnounceRequest,
        peer_addr: CanonicalSocketAddr,
        response_sender: SharedSender<AnnounceResponse>,
    },
    Scrape {
        request: ScrapeRequest,
        peer_addr: CanonicalSocketAddr,
        response_sender: SharedSender<ScrapeResponse>,
    },
}

#[derive(Default, Clone)]
pub struct State {
    pub access_list: Arc<AccessListArcSwap>,
}
