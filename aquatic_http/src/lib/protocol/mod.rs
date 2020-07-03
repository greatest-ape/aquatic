use std::net::IpAddr;
use hashbrown::HashMap;
use serde::{Serialize, Deserialize};

use crate::common::Peer;

mod serde_helpers;

use serde_helpers::*;


#[derive(Debug, Clone, Hash, PartialEq, Eq, Serialize, Deserialize)]
#[serde(transparent)]
pub struct PeerId(
    #[serde(
        deserialize_with = "deserialize_20_bytes",
    )]
    pub [u8; 20]
);


#[derive(Debug, Clone, Hash, PartialEq, Eq, Serialize, Deserialize)]
#[serde(transparent)]
pub struct InfoHash(
    #[serde(
        deserialize_with = "deserialize_20_bytes",
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
    #[serde(
        deserialize_with = "deserialize_bool_from_number",
        default = "AnnounceRequest::default_compact_value"
    )]
    pub compact: bool,
    /// Requested number of peers to return
    pub numwant: usize,
}

impl AnnounceRequest {
    fn default_compact_value() -> bool {
        true
    }
}


#[derive(Debug, Clone, Serialize)]
pub struct AnnounceResponseSuccess {
    #[serde(rename = "interval")]
    pub announce_interval: usize,
    pub tracker_id: String, // Optional??
    pub complete: usize,
    pub incomplete: usize,
    #[serde(
        serialize_with = "serialize_response_peers_compact"
    )]
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
        deserialize_with = "deserialize_info_hashes" // FIXME: does this work?
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


#[derive(Debug, Clone)]
pub enum Request {
    Announce(AnnounceRequest),
    Scrape(ScrapeRequest),
}


impl Request {
    pub fn from_http_get_path(path: &str) -> Option<Self> {
        log::debug!("path: {:?}", path);

        let mut split_parts= path.splitn(2, '?');

        let path = split_parts.next()?;
        let query_string = Self::preprocess_query_string(split_parts.next()?);

        if path == "/announce" {
            let result: Result<AnnounceRequest, serde_urlencoded::de::Error> =
                serde_urlencoded::from_str(&query_string);

            if let Err(ref err) = result {
                log::debug!("error: {}", err);
            }

            result.ok().map(Request::Announce)
        } else {
            let result: Result<ScrapeRequest, serde_urlencoded::de::Error> =
                serde_urlencoded::from_str(&query_string);

            if let Err(ref err) = result {
                log::debug!("error: {}", err);
            }

            result.ok().map(Request::Scrape)
        }
    }

    /// The info hashes and peer id's that are received are url-encoded byte
    /// by byte, e.g., %fa for byte 0xfa. However, they are parsed as an UTF-8
    /// string, meaning that non-ascii bytes are invalid characters. Therefore,
    /// these bytes must be converted to their equivalent multi-byte UTF-8
    /// encodings first.
    fn preprocess_query_string(query_string: &str) -> String {
        let mut processed = String::new();

        for (i, part) in query_string.split('%').enumerate(){
            if i == 0 {
                processed.push_str(part);
            } else if part.len() >= 2 {
                let mut two_first = String::with_capacity(2);
                let mut rest = String::new();

                for (j, c) in part.chars().enumerate(){
                    if j < 2 {
                        two_first.push(c);
                    } else {
                        rest.push(c);
                    }
                }

                let byte = u8::from_str_radix(&two_first, 16).unwrap();

                let mut tmp = [0u8; 4];

                let slice = (byte as char).encode_utf8(&mut tmp);

                for byte in slice.bytes(){
                    processed.push('%');
                    processed.push_str(&format!("{:02x}", byte));
                }

                processed.push_str(&rest);
            }
        }

        processed
    }
}


#[derive(Debug, Clone, Serialize)]
#[serde(untagged)]
pub enum Response {
    AnnounceSuccess(AnnounceResponseSuccess),
    AnnounceFailure(AnnounceResponseFailure),
    Scrape(ScrapeResponse)
}


impl Response {
    pub fn to_bytes(self) -> Vec<u8> {
        bendy::serde::to_bytes(&self).unwrap()
    }
}