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
    pub transaction_id: TransactionId
}


#[derive(PartialEq, Eq, Clone, Debug)]
pub struct AnnounceRequest {
    pub connection_id: ConnectionId,
    pub transaction_id: TransactionId,
    pub info_hash: InfoHash,
    pub peer_id: PeerId,
    pub bytes_downloaded: NumberOfBytes,
    pub bytes_uploaded: NumberOfBytes,
    pub bytes_left: NumberOfBytes,
    pub event: AnnounceEvent,
    pub ip_address: Option<Ipv4Addr>, 
    pub key: PeerKey,
    pub peers_wanted: NumberOfPeers,
    pub port: Port
}


#[derive(PartialEq, Eq, Clone, Debug)]
pub struct ScrapeRequest {
    pub connection_id: ConnectionId,
    pub transaction_id: TransactionId,
    pub info_hashes: Vec<InfoHash>
}


#[derive(PartialEq, Eq, Clone, Debug)]
pub enum Request {
    Connect(ConnectRequest),
    Announce(AnnounceRequest),
    Scrape(ScrapeRequest),
}


impl From<ConnectRequest> for Request {
    fn from(r: ConnectRequest) -> Self {
        Self::Connect(r)
    }
}


impl From<AnnounceRequest> for Request {
    fn from(r: AnnounceRequest) -> Self {
        Self::Announce(r)
    }
}


impl From<ScrapeRequest> for Request {
    fn from(r: ScrapeRequest) -> Self {
        Self::Scrape(r)
    }
}


#[cfg(test)]
impl quickcheck::Arbitrary for AnnounceEvent {
    fn arbitrary<G: quickcheck::Gen>(g: &mut G) -> Self {
        match (bool::arbitrary(g), bool::arbitrary(g)){
            (false, false) => Self::Started,
            (true, false) => Self::Started,
            (false, true) => Self::Completed,
            (true, true) => Self::None,
        }
    }
}


#[cfg(test)]
impl quickcheck::Arbitrary for ConnectRequest {
    fn arbitrary<G: quickcheck::Gen>(g: &mut G) -> Self {
        Self {
            transaction_id: TransactionId(i32::arbitrary(g)),
        }
    }
}


#[cfg(test)]
impl quickcheck::Arbitrary for AnnounceRequest {
    fn arbitrary<G: quickcheck::Gen>(g: &mut G) -> Self {
        Self {
            connection_id: ConnectionId(i64::arbitrary(g)),
            transaction_id: TransactionId(i32::arbitrary(g)),
            info_hash: InfoHash::arbitrary(g),
            peer_id: PeerId::arbitrary(g),
            bytes_downloaded: NumberOfBytes(i64::arbitrary(g)),
            bytes_uploaded: NumberOfBytes(i64::arbitrary(g)),
            bytes_left: NumberOfBytes(i64::arbitrary(g)),
            event: AnnounceEvent::arbitrary(g),
            ip_address: None, 
            key: PeerKey(u32::arbitrary(g)),
            peers_wanted: NumberOfPeers(i32::arbitrary(g)),
            port: Port(u16::arbitrary(g))
        }
    }
}


#[cfg(test)]
impl quickcheck::Arbitrary for ScrapeRequest {
    fn arbitrary<G: quickcheck::Gen>(g: &mut G) -> Self {
        let info_hashes = (0..u8::arbitrary(g)).map(|_| {
            InfoHash::arbitrary(g)
        }).collect();

        Self {
            connection_id: ConnectionId(i64::arbitrary(g)),
            transaction_id: TransactionId(i32::arbitrary(g)),
            info_hashes,
        }
    }
}