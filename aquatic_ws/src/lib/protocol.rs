use std::net::IpAddr;

use hashbrown::HashMap;
use serde::{Serialize, Deserialize};


#[derive(Clone, Hash, PartialEq, Eq, Serialize, Deserialize)]
pub struct PeerId(pub [u8; 20]);


#[derive(Clone, Hash, PartialEq, Eq, Serialize, Deserialize)]
pub struct InfoHash(pub [u8; 20]);


#[derive(Clone, Serialize)]
pub struct ResponsePeer {
    pub peer_id: PeerId,
    pub ip: IpAddr, // From src socket addr
    pub port: u16, // From src port
    pub complete: bool, // bytes_left == 0
}


#[derive(Clone, Deserialize)]
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
#[derive(Clone, Serialize)]
pub struct MiddlemanOfferToPeer {
    pub peer_id: PeerId, // Peer id of peer sending offer
    pub info_hash: InfoHash,
    pub offer: (), // Gets copied from AnnounceRequestOffer
    pub offer_id: (), // Gets copied from AnnounceRequestOffer
}


/// If announce request has answer = true, send this to peer with
/// peer id == "to_peer_id" field
/// Action field should be 'announce'
#[derive(Clone, Serialize)]
pub struct MiddlemanAnswerToPeer {
    pub peer_id: PeerId,
    pub info_hash: InfoHash,
    pub answer: bool,
    pub offer_id: (),
}


/// Element of AnnounceRequest.offers
#[derive(Clone, Deserialize)]
pub struct AnnounceRequestOffer {
    pub offer: (), // TODO: Check client for what this is
    pub offer_id: (), // TODO: Check client for what this is
}


#[derive(Clone, Deserialize)]
pub struct AnnounceRequest {
    pub info_hash: InfoHash, // FIXME: I think these are actually really just strings with 20 len, same with peer id
    pub peer_id: PeerId,
    #[serde(rename = "left")]
    pub bytes_left: bool, // Just called "left" in protocol
    #[serde(default)]
    pub event: AnnounceEvent, // Can be empty? Then, default is "update"

    // Length of this is number of peers wanted?
    // Only when this is an array offers are sent
    pub offers: Option<Vec<AnnounceRequestOffer>>, 

    /// If false, send response before sending offers (or possibly "skip sending update back"?)
    /// If true, send MiddlemanAnswerToPeer to peer with "to_peer_id" as peer_id.
    #[serde(default)]
    pub answer: bool, 
    pub to_peer_id: Option<PeerId>, // Only parsed to hex if answer == true, probably undefined otherwise
}


#[derive(Clone, Serialize)]
pub struct AnnounceResponse {
    pub info_hash: InfoHash,
    pub complete: usize,
    pub incomplete: usize,
    // I suspect receivers don't care about this and rely on offers instead??
    // Also, what does it contain, exacly (not certain that it is ResponsePeer?)
    pub peers: Vec<ResponsePeer>,
    #[serde(rename = "interval")]
    pub announce_interval: usize, // Default 2 min probably

    // Sent to "to_peer_id" peer (?? or did I put this into MiddlemanAnswerToPeer instead?)
    // pub offer_id: (),
    // pub answer: bool,
}


#[derive(Clone, Deserialize)]
pub struct ScrapeRequest {
    // If omitted, scrape for all torrents, apparently
    // There is some kind of parsing here too which accepts a single info hash
    // and puts it into a vector
    pub info_hashes: Option<Vec<InfoHash>>,
}


#[derive(Clone, Serialize)]
pub struct ScrapeStatistics {
    pub complete: usize,
    pub incomplete: usize,
    pub downloaded: usize,
}


#[derive(Clone, Serialize)]
pub struct ScrapeResponse {
    pub files: HashMap<InfoHash, ScrapeStatistics>, // InfoHash to Scrape stats
    pub flags: HashMap<String, usize>,
}


#[derive(Clone, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Action {
    Announce,
    Scrape
}


#[derive(Clone, Serialize, Deserialize)]
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

        if let Ok(wrapper) = res {
            if let Action::Announce = wrapper.action {
                return Some(InMessage::AnnounceRequest(wrapper.inner));
            }
        }
        
        let res: Result<ActionWrapper<ScrapeRequest>, _> = serde_json::from_str(&text);

        if let Ok(wrapper) = res {
            if let Action::Scrape = wrapper.action {
                return Some(InMessage::ScrapeRequest(wrapper.inner));
            }
        }

        None
    }
}


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