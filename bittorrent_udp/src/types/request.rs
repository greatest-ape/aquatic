use std::net::Ipv4Addr;

use super::common::*;


#[derive(PartialEq, Eq, Hash, Clone, Copy, Debug)]
pub enum AnnounceEvent {
    Started,
    Stopped,
    Completed,
    None
}

#[derive(PartialEq, Eq, Clone, Debug)]
pub struct ConnectRequest {
    pub transaction_id:   TransactionId
}

#[derive(PartialEq, Eq, Clone, Debug)]
pub struct AnnounceRequest {
    pub connection_id:    ConnectionId,
    pub transaction_id:   TransactionId,
    pub info_hash:        InfoHash,
    pub peer_id:          PeerId,
    pub bytes_downloaded: NumberOfBytes,
    pub bytes_uploaded:   NumberOfBytes,
    pub bytes_left:       NumberOfBytes,
    pub event:            AnnounceEvent,
    pub ip_address:       Option<Ipv4Addr>, 
    pub key:              PeerKey,
    pub peers_wanted:     NumberOfPeers,
    pub port:             Port
}

#[derive(PartialEq, Eq, Clone, Debug)]
pub struct ScrapeRequest {
    pub connection_id:    ConnectionId,
    pub transaction_id:   TransactionId,
    pub info_hashes:      Vec<InfoHash>
}

/// This is used for returning specific errors from the parser
#[derive(PartialEq, Eq, Clone, Debug)]
pub struct InvalidRequest { 
    pub transaction_id:   TransactionId,
    pub message:          String
}

#[derive(PartialEq, Eq, Clone, Debug)]
pub enum Request {
    Connect(ConnectRequest),
    Announce(AnnounceRequest),
    Scrape(ScrapeRequest),
    Invalid(InvalidRequest),
}