use std::net::SocketAddr;

use aquatic_http_protocol::{
    request::{AnnounceRequest, ScrapeRequest},
    response::{AnnounceResponse, ScrapeResponse},
};

#[derive(Copy, Clone, Debug)]
pub struct ConsumerId(pub usize);

#[derive(Clone, Copy, Debug)]
pub struct ConnectionId(pub usize);

#[derive(Debug)]
pub enum ChannelRequest {
    Announce {
        request: AnnounceRequest,
        peer_addr: SocketAddr,
        connection_id: ConnectionId,
        response_consumer_id: ConsumerId,
    },
    Scrape {
        request: ScrapeRequest,
        peer_addr: SocketAddr,
        connection_id: ConnectionId,
        response_consumer_id: ConsumerId,
    },
}

#[derive(Debug)]
pub enum ChannelResponse {
    Announce {
        response: AnnounceResponse,
        peer_addr: SocketAddr,
        connection_id: ConnectionId,
    },
    Scrape {
        response: ScrapeResponse,
        peer_addr: SocketAddr,
        connection_id: ConnectionId,
    },
}

impl ChannelResponse {
    pub fn get_connection_id(&self) -> ConnectionId {
        match self {
            Self::Announce { connection_id, .. } => *connection_id,
            Self::Scrape { connection_id, .. } => *connection_id,
        }
    }
    pub fn get_peer_addr(&self) -> SocketAddr {
        match self {
            Self::Announce { peer_addr, .. } => *peer_addr,
            Self::Scrape { peer_addr, .. } => *peer_addr,
        }
    }
}
