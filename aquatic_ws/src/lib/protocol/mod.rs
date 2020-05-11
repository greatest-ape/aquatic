use hashbrown::HashMap;
use serde::{Serialize, Deserialize};

pub mod deserialize;

use deserialize::*;


#[derive(Debug, Clone, Hash, PartialEq, Eq, Serialize, Deserialize)]
#[serde(transparent)]
pub struct PeerId(
    #[serde(deserialize_with = "deserialize_20_bytes")]
    pub [u8; 20]
);


#[derive(Debug, Clone, Hash, PartialEq, Eq, Serialize, Deserialize)]
#[serde(transparent)]
pub struct InfoHash(
    #[serde(deserialize_with = "deserialize_20_bytes")]
    pub [u8; 20]
);


#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(transparent)]
pub struct OfferId(
    #[serde(deserialize_with = "deserialize_20_bytes")]
    pub [u8; 20]
);


/// Some kind of nested structure from https://www.npmjs.com/package/simple-peer
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(transparent)]
pub struct JsonValue(pub ::serde_json::Value);


#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum AnnounceEvent {
    Started,
    Stopped,
    Completed,
    Update
}


impl Default for AnnounceEvent {
    fn default() -> Self {
        Self::Update
    }
}


/// Apparently, these are sent to a number of peers when they are set
/// in an AnnounceRequest
/// action = "announce"
#[derive(Debug, Clone, Serialize)]
pub struct MiddlemanOfferToPeer {
    /// Peer id of peer sending offer
    /// Note: if equal to client peer_id, client ignores offer
    pub peer_id: PeerId,
    pub info_hash: InfoHash,
    /// Gets copied from AnnounceRequestOffer
    pub offer: JsonValue, 
    /// Gets copied from AnnounceRequestOffer
    pub offer_id: OfferId,
}


/// If announce request has answer = true, send this to peer with
/// peer id == "to_peer_id" field
/// Action field should be 'announce'
#[derive(Debug, Clone, Serialize)]
pub struct MiddlemanAnswerToPeer {
    /// Note: if equal to client peer_id, client ignores answer
    pub peer_id: PeerId,
    pub info_hash: InfoHash,
    pub answer: JsonValue,
    pub offer_id: OfferId,
}


/// Element of AnnounceRequest.offers
#[derive(Debug, Clone, Deserialize)]
pub struct AnnounceRequestOffer {
    pub offer: JsonValue,
    pub offer_id: OfferId,
}


#[derive(Debug, Clone, Deserialize)]
pub struct AnnounceRequest {
    pub info_hash: InfoHash,
    pub peer_id: PeerId,
    /// Just called "left" in protocol
    #[serde(rename = "left")]
    pub bytes_left: Option<usize>,
    /// Can be empty. Then, default is "update"
    #[serde(default)]
    pub event: AnnounceEvent,

    /// Only when this is an array offers are sent to random peers
    /// Length of this is number of peers wanted?
    /// Max length of this is 10 in reference client code
    /// Not sent when announce event is stopped or completed
    pub offers: Option<Vec<AnnounceRequestOffer>>, 
    /// Seems to only get sent by client when sending offers, and is also same
    /// as length of offers vector (or at least never less)
    /// Max length of this is 10 in reference client code
    /// Could probably be ignored, `offers.len()` should provide needed info
    pub numwant: Option<usize>,

    /// If empty, send response before sending offers (or possibly "skip sending update back"?)
    /// Else, send MiddlemanAnswerToPeer to peer with "to_peer_id" as peer_id.
    /// I think using Option is good, it seems like this isn't always set
    /// (same as `offers`)
    pub answer: Option<JsonValue>, 
    /// Likely undefined if !(answer == true)
    pub to_peer_id: Option<PeerId>, 
    /// Sent if answer is set
    pub offer_id: Option<OfferId>,
}


#[derive(Debug, Clone, Serialize)]
pub struct AnnounceResponse {
    pub info_hash: InfoHash,
    /// Client checks if this is null, not clear why
    pub complete: usize,
    pub incomplete: usize,
    #[serde(rename = "interval")]
    pub announce_interval: usize, // Default 2 min probably
}


#[derive(Debug, Clone, Deserialize)]
pub struct ScrapeRequest {
    // If omitted, scrape for all torrents, apparently
    // There is some kind of parsing here too which accepts a single info hash
    // and puts it into a vector
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
    // Looks like `flags` field is ignored in reference client
    // pub flags: HashMap<String, usize>,
}


#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Action {
    Announce,
    Scrape
}


/// Helper for serializing and deserializing messages
#[derive(Debug, Clone, Serialize, Deserialize)]
struct ActionWrapper<T> {
    pub action: Action,
    #[serde(flatten)]
    pub inner: T,
}


impl <T>ActionWrapper<T> {
    pub fn announce(t: T) -> Self {
        Self {
            action: Action::Announce,
            inner: t
        }
    }
    pub fn scrape(t: T) -> Self {
        Self {
            action: Action::Scrape,
            inner: t
        }
    }
}


#[derive(Debug, Clone)]
pub enum InMessage {
    AnnounceRequest(AnnounceRequest),
    ScrapeRequest(ScrapeRequest),
}


impl InMessage {
    /// Try parsing as announce request first. If that fails, try parsing as
    /// scrape request, or return None
    pub fn from_ws_message(ws_message: tungstenite::Message) -> Option<Self> {
        use tungstenite::Message::{Text, Binary};

        let text = match ws_message {
            Text(text) => Some(text),
            Binary(bytes) => String::from_utf8(bytes).ok(),
            _ => None
        }?;

        let res: Result<ActionWrapper<AnnounceRequest>, _> = serde_json::from_str(&text);

        if let Ok(ActionWrapper { action: Action::Announce, inner }) = res {
            return Some(InMessage::AnnounceRequest(inner));
        } else {
            dbg!(res);
        }
        
        let res: Result<ActionWrapper<ScrapeRequest>, _> = serde_json::from_str(&text);

        if let Ok(ActionWrapper { action: Action::Scrape, inner }) = res {
            return Some(InMessage::ScrapeRequest(inner));
        }

        None
    }
}


#[derive(Debug, Clone)]
pub enum OutMessage {
    AnnounceResponse(AnnounceResponse),
    ScrapeResponse(ScrapeResponse),
    Offer(MiddlemanOfferToPeer),
    Answer(MiddlemanAnswerToPeer),
}


impl OutMessage {
    pub fn to_ws_message(self) -> tungstenite::Message {
        let json = match self {
            Self::AnnounceResponse(message) => {
                serde_json::to_string(
                    &ActionWrapper::announce(message)
                ).unwrap()
            },
            Self::Offer(message) => {
                serde_json::to_string(
                    &ActionWrapper::announce(message)
                ).unwrap()
            },
            Self::Answer(message) => {
                serde_json::to_string(
                    &ActionWrapper::announce(message)
                ).unwrap()
            },
            Self::ScrapeResponse(message) => {
                serde_json::to_string(
                    &ActionWrapper::scrape(message)
                ).unwrap()
            },
        };
                
        tungstenite::Message::from(json)
    }
}