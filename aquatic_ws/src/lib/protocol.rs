use std::collections::HashMap;
use std::net::IpAddr;


/// TODO: will need to store socket worker index and connection index
/// for middleman activities, also save SocketAddr instead of IP and port,
/// maybe, for comparison in socket worker
pub struct Peer {
    pub complete: bool, // bytes_left == 0
    pub peer_id: [u8; 20],
    pub ip: IpAddr, // From src socket addr
    pub port: u16, // From src port
}


pub enum AnnounceEvent {
    Started,
    Stopped,
    Completed,
    Update
}


/// Apparently, these are sent to a number of peers when they are set
/// in an AnnounceRequest
/// action = "announce"
pub struct MiddlemanOfferToPeer {
    pub offer: (), // Gets copied from AnnounceRequestOffer
    pub offer_id: (), // Gets copied from AnnounceRequestOffer
    pub peer_id: [u8; 20], // Peer id of peer sending offer
    pub info_hash: [u8; 20],
}


/// If announce request has answer = true, send this to peer with
/// peer id == "to_peer_id" field
/// Action field should be 'announce'
pub struct MiddlemanAnswerToPeer {
    pub answer: bool,
    pub offer_id: (),
    pub peer_id: [u8; 20],
    pub info_hash: [u8; 20],
}


/// Element of AnnounceRequest.offers
pub struct AnnounceRequestOffer {
    pub offer: (), // TODO: Check client for what this is
    pub offer_id: (), // TODO: Check client for what this is
}


pub struct AnnounceRequest {
    pub info_hash: [u8; 20], // FIXME: I think these are actually really just strings with 20 len, same with peer id
    pub peer_id: [u8; 20],
    pub bytes_left: bool, // Just called "left" in protocol
    pub event: AnnounceEvent, // Can be empty? Then, default is "update"

    // Length of this is number of peers wanted?
    // Only when this is an array offers are sent
    pub offers: Option<Vec<AnnounceRequestOffer>>, 

    /// If false, send response before sending offers (or possibly "skip sending update back"?)
    /// If true, send MiddlemanAnswerToPeer to peer with "to_peer_id" as peer_id.
    pub answer: bool, 
    pub to_peer_id: Option<[u8; 20]>, // Only parsed to hex if answer == true, probably undefined otherwise
}


pub struct AnnounceResponse {
    pub info_hash: [u8; 20],
    pub complete: usize,
    pub incomplete: usize,
    // I suspect receivers don't care about this and rely on offers instead??
    // Also, what does it contain, exacly?
    pub peers: Vec<()>,
    pub interval: usize, // Default 2 min probably

    // Sent to "to_peer_id" peer (?? or did I put this into MiddlemanAnswerToPeer instead?)
    pub offer_id: (),
    pub answer: bool,
}



pub struct ScrapeRequest {
    // If omitted, scrape for all torrents, apparently
    // There is some kind of parsing here too which accepts a single info hash
    // and puts it into a vector
    pub info_hashes: Option<Vec<[u8; 20]>>,
}


pub struct ScrapeStatistics {
    pub complete: usize,
    pub incomplete: usize,
    pub downloaded: usize,
}


pub struct ScrapeResponse {
    pub files: HashMap<[u8; 20], ScrapeStatistics>, // InfoHash to Scrape stats
    pub flags: HashMap<String, usize>,
}
//pub struct ScrapeResponse {
//    pub complete: usize,
//    pub incomplete: usize,
//}