use std::{net::IpAddr, sync::Arc};

use aquatic_common::access_list::AccessListArcSwap;

pub use aquatic_common::ValidUntil;
use aquatic_ws_protocol::common::{InfoHash, PeerId};

#[derive(Copy, Clone, Debug)]
pub enum IpVersion {
    V4,
    V6,
}

impl IpVersion {
    pub fn canonical_from_ip(ip: IpAddr) -> IpVersion {
        match ip {
            IpAddr::V4(_) => Self::V4,
            IpAddr::V6(addr) => match addr.octets() {
                [0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0xff, 0xff, _, _, _, _] => Self::V4,
                _ => Self::V6,
            },
        }
    }
}

#[derive(Default, Clone)]
pub struct State {
    pub access_list: Arc<AccessListArcSwap>,
}

#[derive(Copy, Clone, Debug)]
pub struct PendingScrapeId(pub u8);

#[derive(Copy, Clone, Debug)]
pub struct ConsumerId(pub u8);

slotmap::new_key_type! {
    pub struct ConnectionId;
}

#[derive(Clone, Copy, Debug)]
pub struct InMessageMeta {
    /// Index of socket worker responsible for this connection. Required for
    /// sending back response through correct channel to correct worker.
    pub out_message_consumer_id: ConsumerId,
    pub connection_id: ConnectionId,
    pub ip_version: IpVersion,
    pub pending_scrape_id: Option<PendingScrapeId>,
}

#[derive(Clone, Copy, Debug)]
pub struct OutMessageMeta {
    /// Index of socket worker responsible for this connection. Required for
    /// sending back response through correct channel to correct worker.
    pub out_message_consumer_id: ConsumerId,
    pub connection_id: ConnectionId,
    pub pending_scrape_id: Option<PendingScrapeId>,
}

impl From<InMessageMeta> for OutMessageMeta {
    fn from(val: InMessageMeta) -> Self {
        OutMessageMeta {
            out_message_consumer_id: val.out_message_consumer_id,
            connection_id: val.connection_id,
            pending_scrape_id: val.pending_scrape_id,
        }
    }
}

#[derive(Clone, Debug)]
pub enum SwarmControlMessage {
    ConnectionClosed {
        ip_version: IpVersion,
        announced_info_hashes: Vec<(InfoHash, PeerId)>,
    },
}
