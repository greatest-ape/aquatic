use std::net::IpAddr;

use hashbrown::HashMap;
use serde::Serialize;

use crate::common::Peer;

use super::common::*;
use super::utils::*;


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


#[derive(Debug, Clone, Serialize)]
pub struct ScrapeStatistics {
    pub complete: usize,
    pub incomplete: usize,
    pub downloaded: usize,
}


#[derive(Debug, Clone, Serialize)]
pub struct AnnounceResponse {
    #[serde(rename = "interval")]
    pub announce_interval: usize,
    pub complete: usize,
    pub incomplete: usize,
    #[serde(
        serialize_with = "serialize_response_peers_compact"
    )]
    pub peers: Vec<ResponsePeer>,
}


#[derive(Debug, Clone, Serialize)]
pub struct ScrapeResponse {
    pub files: HashMap<InfoHash, ScrapeStatistics>,
}


#[derive(Debug, Clone, Serialize)]
pub struct FailureResponse {
    pub failure_reason: String,
}


#[derive(Debug, Clone, Serialize)]
#[serde(untagged)]
pub enum Response {
    Announce(AnnounceResponse),
    Scrape(ScrapeResponse),
    Failure(FailureResponse),
}


impl Response {
    pub fn to_bytes(&self) -> Vec<u8> {
        match bendy::serde::to_bytes(self){
            Ok(bytes) => bytes,
            Err(err) => {
                log::error!("error encoding response: {}", err);

                Vec::new()
            }
        }
    }
}