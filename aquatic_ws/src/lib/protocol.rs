use std::net::IpAddr;

use hashbrown::HashMap;


pub struct PeerId(pub [u8; 20]);


pub struct InfoHash(pub [u8; 20]);


pub struct ResponsePeer {
    pub peer_id: PeerId,
    pub ip: IpAddr, // From src socket addr
    pub port: u16, // From src port
    pub complete: bool, // bytes_left == 0
}


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
pub struct MiddlemanOfferToPeer {
    pub peer_id: PeerId, // Peer id of peer sending offer
    pub info_hash: InfoHash,
    pub offer: (), // Gets copied from AnnounceRequestOffer
    pub offer_id: (), // Gets copied from AnnounceRequestOffer
}


/// If announce request has answer = true, send this to peer with
/// peer id == "to_peer_id" field
/// Action field should be 'announce'
pub struct MiddlemanAnswerToPeer {
    pub peer_id: PeerId,
    pub info_hash: InfoHash,
    pub answer: bool,
    pub offer_id: (),
}


/// Element of AnnounceRequest.offers
pub struct AnnounceRequestOffer {
    pub offer: (), // TODO: Check client for what this is
    pub offer_id: (), // TODO: Check client for what this is
}


pub struct AnnounceRequest {
    pub info_hash: InfoHash, // FIXME: I think these are actually really just strings with 20 len, same with peer id
    pub peer_id: PeerId,
    pub bytes_left: bool, // Just called "left" in protocol
    pub event: AnnounceEvent, // Can be empty? Then, default is "update"

    // Length of this is number of peers wanted?
    // Only when this is an array offers are sent
    pub offers: Option<Vec<AnnounceRequestOffer>>, 

    /// If false, send response before sending offers (or possibly "skip sending update back"?)
    /// If true, send MiddlemanAnswerToPeer to peer with "to_peer_id" as peer_id.
    pub answer: bool, 
    pub to_peer_id: Option<PeerId>, // Only parsed to hex if answer == true, probably undefined otherwise
}


pub struct AnnounceResponse {
    pub info_hash: InfoHash,
    pub complete: usize,
    pub incomplete: usize,
    // I suspect receivers don't care about this and rely on offers instead??
    // Also, what does it contain, exacly (not certain that it is ResponsePeer?)
    pub peers: Vec<ResponsePeer>,
    pub interval: usize, // Default 2 min probably

    // Sent to "to_peer_id" peer (?? or did I put this into MiddlemanAnswerToPeer instead?)
    // pub offer_id: (),
    // pub answer: bool,
}


pub struct ScrapeRequest {
    // If omitted, scrape for all torrents, apparently
    // There is some kind of parsing here too which accepts a single info hash
    // and puts it into a vector
    pub info_hashes: Option<Vec<InfoHash>>,
}


pub struct ScrapeStatistics {
    pub complete: usize,
    pub incomplete: usize,
    pub downloaded: usize,
}


pub struct ScrapeResponse {
    pub files: HashMap<InfoHash, ScrapeStatistics>, // InfoHash to Scrape stats
    pub flags: HashMap<String, usize>,
}


pub enum InMessage {
    AnnounceRequest(AnnounceRequest),
    ScrapeRequest(ScrapeRequest),
}


impl InMessage {
    fn from_ws_message(ws_messge: tungstenite::Message) -> Result<Self, ()> {
        unimplemented!()
    }
}


pub enum OutMessage {
    AnnounceResponse(AnnounceResponse),
    ScrapeResponse(ScrapeResponse),
    Offer(MiddlemanOfferToPeer),
    Answer(MiddlemanAnswerToPeer),
}


impl OutMessage {
    fn to_ws_message(self) -> tungstenite::Message {
        unimplemented!()
    }
}