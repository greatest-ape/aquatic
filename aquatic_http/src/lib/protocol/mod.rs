use std::net::IpAddr;
use hashbrown::HashMap;
use serde::{Serialize, Deserialize};

use crate::common::Peer;

mod serde_helpers;

use serde_helpers::*;


#[derive(Debug, Clone, Copy, Hash, PartialEq, Eq, Serialize, Deserialize)]
#[serde(transparent)]
pub struct PeerId(
    #[serde(
        deserialize_with = "deserialize_20_bytes",
        serialize_with = "serialize_20_bytes"
    )]
    pub [u8; 20]
);


#[derive(Debug, Clone, Copy, Hash, PartialEq, Eq, Serialize, Deserialize)]
#[serde(transparent)]
pub struct InfoHash(
    #[serde(
        deserialize_with = "deserialize_20_bytes",
        serialize_with = "serialize_20_bytes"
    )]
    pub [u8; 20]
);


#[derive(Clone, Copy, Debug, Serialize)]
pub struct ResponsePeer {
    pub ip_address: IpAddr,
    pub port: u16
}


impl ResponsePeer {
    pub fn from_peer(peer: &Peer) -> Self {
        let ip_address = peer.connection_meta.peer_addr.ip();

        Self {
            ip_address,
            port: peer.port
        }
    }
}


#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum AnnounceEvent {
    Started,
    Stopped,
    Completed,
    Empty
}


impl Default for AnnounceEvent {
    fn default() -> Self {
        Self::Empty
    }
}


#[derive(Debug, Clone, Deserialize)]
pub struct AnnounceRequest {
    pub info_hash: InfoHash,
    pub peer_id: PeerId,
    pub port: u16,
    #[serde(rename = "left")]
    pub bytes_left: usize,
    #[serde(default)]
    pub event: AnnounceEvent,
    /// FIXME: number: 0 or 1
    pub compact: bool,
    /// Requested number of peers to return
    pub numwant: usize,
}


#[derive(Debug, Clone, Serialize)]
pub struct AnnounceResponseSuccess {
    #[serde(rename = "interval")]
    pub announce_interval: usize,
    pub tracker_id: String, // Optional??
    pub complete: usize,
    pub incomplete: usize,
    pub peers: Vec<ResponsePeer>,
}


#[derive(Debug, Clone, Serialize)]
pub struct AnnounceResponseFailure {
    pub failure_reason: String,
}


#[derive(Debug, Clone, Deserialize)]
pub struct ScrapeRequest {
    #[serde(
        rename = "info_hash",
        deserialize_with = "deserialize_info_hashes",
        default
    )]
    pub info_hashes: Vec<InfoHash>,
}


#[derive(Debug, Clone, Serialize)]
pub struct ScrapeStatistics {
    pub complete: usize,
    pub incomplete: usize,
    pub downloaded: usize,
}


#[derive(Debug, Clone, Serialize)]
pub struct ScrapeResponse {
    pub files: HashMap<InfoHash, ScrapeStatistics>,
}


#[derive(Debug, Clone, Deserialize)]
pub enum Request {
    Announce(AnnounceRequest),
    Scrape(ScrapeRequest),
}


impl Request {
    pub fn from_http() -> Self {
        unimplemented!()
    }
}


#[derive(Debug, Clone, Serialize)]
pub enum Response {
    AnnounceSuccess(AnnounceResponseSuccess),
    AnnounceFailure(AnnounceResponseFailure),
    Scrape(ScrapeResponse)
}