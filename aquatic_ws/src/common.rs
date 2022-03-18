use std::sync::Arc;

use aquatic_common::access_list::AccessListArcSwap;
use aquatic_common::CanonicalSocketAddr;

pub use aquatic_common::ValidUntil;

pub type TlsConfig = futures_rustls::rustls::ServerConfig;

#[derive(Default, Clone)]
pub struct State {
    pub access_list: Arc<AccessListArcSwap>,
}

#[derive(Copy, Clone, Debug)]
pub struct PendingScrapeId(pub usize);

#[derive(Copy, Clone, Debug)]
pub struct ConsumerId(pub usize);

#[derive(Clone, Copy, Debug)]
pub struct ConnectionId(pub usize);

#[derive(Clone, Copy, Debug)]
pub struct ConnectionMeta {
    /// Index of socket worker responsible for this connection. Required for
    /// sending back response through correct channel to correct worker.
    pub out_message_consumer_id: ConsumerId,
    pub connection_id: ConnectionId,
    pub peer_addr: CanonicalSocketAddr,
    pub pending_scrape_id: Option<PendingScrapeId>,
}
