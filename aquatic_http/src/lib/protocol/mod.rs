use std::net::IpAddr;
use std::str::FromStr;

use anyhow::Context;
use hashbrown::HashMap;
use serde::Serialize;

use crate::common::Peer;

mod serde_helpers;

use serde_helpers::*;


#[derive(Debug, Clone, Hash, PartialEq, Eq, Serialize)]
#[serde(transparent)]
pub struct PeerId(
    #[serde(
        serialize_with = "serialize_20_bytes",
    )]
    pub [u8; 20]
);


#[derive(Debug, Clone, Hash, PartialEq, Eq, Serialize)]
#[serde(transparent)]
pub struct InfoHash(
    #[serde(
        serialize_with = "serialize_20_bytes",
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


#[derive(Debug, Clone)]
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


impl FromStr for AnnounceEvent {
    type Err = String;

    fn from_str(value: &str) -> std::result::Result<Self, String> {
        let event = match value {
            "started" => Self::Started,
            "stopped" => Self::Stopped,
            "completed" => Self::Completed,
            _ => Self::default(),
        };

        Ok(event)
    }
}


#[derive(Debug, Clone)]
pub struct AnnounceRequest {
    pub info_hash: InfoHash,
    pub peer_id: PeerId,
    pub port: u16,
    pub bytes_left: usize,
    pub event: AnnounceEvent,
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
    #[serde(
        serialize_with = "serialize_response_peers_compact"
    )]
    pub peers: Vec<ResponsePeer>,
}


#[derive(Debug, Clone, Serialize)]
pub struct AnnounceResponseFailure {
    pub failure_reason: String,
}


#[derive(Debug, Clone)]
pub struct ScrapeRequest {
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
    /// Parse Request from http path (GET `/announce?info_hash=...`)
    ///
    /// Existing serde-url decode crates were insufficient, so the decision was
    /// made to create a custom parser. serde_urlencoded doesn't support multiple
    /// values with same key, and serde_qs pulls in lots of dependencies. Both
    /// would need preprocessing for the binary format used for info_hash and
    /// peer_id.
    pub fn from_http_get_path(path: &str) -> anyhow::Result<Self> {
        let mut split_parts= path.splitn(2, '?');

        let location = split_parts.next()
            .with_context(|| "no location")?;
        let query_string = split_parts.next()
            .with_context(|| "no query string")?;

        let mut info_hashes = Vec::new();
        let mut data = HashMap::new();

        for part in query_string.split('&'){
            let mut key_and_value = part.splitn(2, '=');

            let key = key_and_value.next()
                .with_context(|| format!("no key in {}", part))?;
            let value = key_and_value.next()
                .with_context(|| format!("no value in {}", part))?;
            let value = Self::urldecode(value).to_string();

            if key == "info_hash" {
                info_hashes.push(value);
            } else {
                data.insert(key, value);
            }
        }

        if location == "/announce" {
            let request = AnnounceRequest {
                info_hash: info_hashes.get(0)
                    .with_context(|| "no info_hash")
                    .and_then(|s| deserialize_20_bytes(s))
                    .map(InfoHash)?,
                peer_id: data.get("peer_id")
                    .with_context(|| "no peer_id")
                    .and_then(|s| deserialize_20_bytes(s))
                    .map(PeerId)?,
                port: data.get("port")
                    .with_context(|| "no port")
                    .and_then(|s| s.parse()
                    .map_err(|err| anyhow::anyhow!("parse 'port': {}", err)))?,
                bytes_left: data.get("left")
                    .with_context(|| "no left")
                    .and_then(|s| s.parse()
                    .map_err(|err| anyhow::anyhow!("parse 'left': {}", err)))?,
                event: data.get("event")
                    .and_then(|s| s.parse().ok())
                    .unwrap_or_default(),
                compact: data.get("compact")
                    .map(|s| s == "1")
                    .unwrap_or(true),
                numwant: data.get("numwant")
                    .with_context(|| "no numwant")
                    .and_then(|s| s.parse()
                    .map_err(|err|
                        anyhow::anyhow!("parse 'numwant': {}", err)
                    ))?,
            };

            Ok(Request::Announce(request))
        } else {
            let mut parsed_info_hashes = Vec::with_capacity(info_hashes.len());

            for info_hash in info_hashes {
                parsed_info_hashes.push(InfoHash(deserialize_20_bytes(&info_hash)?));
            }

            let request = ScrapeRequest {
                info_hashes: parsed_info_hashes,
            };

            Ok(Request::Scrape(request))
        }
    }

    /// The info hashes and peer id's that are received are url-encoded byte
    /// by byte, e.g., %fa for byte 0xfa. However, they need to be parsed as
    /// UTF-8 string, meaning that non-ascii bytes are invalid characters.
    /// Therefore, these bytes must be converted to their equivalent multi-byte
    /// UTF-8 encodings.
    fn urldecode(value: &str) -> String {
        let mut processed = String::new();

        for (i, part) in value.split('%').enumerate(){
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

                processed.push(byte as char);
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
        match bendy::serde::to_bytes(&self){
            Ok(bytes) => bytes,
            Err(err) => {
                log::error!("error encoding response: {}", err);

                Vec::new()
            }
        }
    }
}