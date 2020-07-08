use std::net::{IpAddr, Ipv4Addr, Ipv6Addr};

use hashbrown::HashMap;
use serde::Serialize;

use crate::common::Peer;

use super::common::*;
use super::utils::*;


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


#[derive(Clone, Copy, Debug, Serialize)]
pub struct ResponsePeerV4 {
    pub ip_address: Ipv4Addr,
    pub port: u16
}


#[derive(Clone, Copy, Debug, Serialize)]
pub struct ResponsePeerV6 {
    pub ip_address: Ipv6Addr,
    pub port: u16
}


#[derive(Debug, Clone, Serialize)]
#[serde(transparent)]
pub struct ResponsePeerListV4(
    #[serde(serialize_with = "serialize_response_peers_ipv4")]
    pub Vec<ResponsePeerV4>
);


#[derive(Debug, Clone, Serialize)]
#[serde(transparent)]
pub struct ResponsePeerListV6(
    #[serde(serialize_with = "serialize_response_peers_ipv6")]
    pub Vec<ResponsePeerV6>
);


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
    pub peers: ResponsePeerListV4,
    pub peers6: ResponsePeerListV6,
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