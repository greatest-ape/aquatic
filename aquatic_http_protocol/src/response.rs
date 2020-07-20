use std::net::{Ipv4Addr, Ipv6Addr};

use std::collections::BTreeMap;
use serde::Serialize;

use super::common::*;
use super::utils::*;


#[derive(Debug, Clone, Serialize)]
pub struct ResponsePeer<I>{
    pub ip_address: I,
    pub port: u16
}


#[derive(Debug, Clone, Serialize)]
#[serde(transparent)]
pub struct ResponsePeerListV4(
    #[serde(serialize_with = "serialize_response_peers_ipv4")]
    pub Vec<ResponsePeer<Ipv4Addr>>
);


#[derive(Debug, Clone, Serialize)]
#[serde(transparent)]
pub struct ResponsePeerListV6(
    #[serde(serialize_with = "serialize_response_peers_ipv6")]
    pub Vec<ResponsePeer<Ipv6Addr>>
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


impl AnnounceResponse {
    fn to_bytes(&self) -> Vec<u8> {
        let peers_bytes_len = self.peers.0.len() * 6;
        let peers6_bytes_len = self.peers6.0.len() * 18;

        let mut bytes = Vec::with_capacity(
            12 +
            5 + // Upper estimate
            15 + 
            5 + // Upper estimate
            12 +
            5 + // Upper estimate
            8 +
            peers_bytes_len +
            8 +
            peers6_bytes_len +
            1
        );

        bytes.extend_from_slice(b"d8:completei");
        let _ = itoa::write(&mut bytes, self.complete);

        bytes.extend_from_slice(b"e10:incompletei");
        let _ = itoa::write(&mut bytes, self.incomplete);

        bytes.extend_from_slice(b"e8:intervali");
        let _ = itoa::write(&mut bytes, self.announce_interval);

        bytes.extend_from_slice(b"e5:peers");
        let _ = itoa::write(&mut bytes, peers_bytes_len);
        bytes.push(b':');
        for peer in self.peers.0.iter() {
            bytes.extend_from_slice(&u32::from(peer.ip_address).to_be_bytes());
            bytes.extend_from_slice(&peer.port.to_be_bytes())
        }

        bytes.extend_from_slice(b"6:peers6");
        let _ = itoa::write(&mut bytes, peers6_bytes_len);
        bytes.push(b':');
        for peer in self.peers6.0.iter() {
            bytes.extend_from_slice(&u128::from(peer.ip_address).to_be_bytes());
            bytes.extend_from_slice(&peer.port.to_be_bytes())
        }
        bytes.push(b'e');

        bytes
    }
}


#[derive(Debug, Clone, Serialize)]
pub struct ScrapeResponse {
    /// BTreeMap instead of HashMap since keys need to be serialized in order
    pub files: BTreeMap<InfoHash, ScrapeStatistics>,
}


impl ScrapeResponse {
    fn to_bytes(&self) -> Vec<u8> {
        let mut bytes = Vec::with_capacity(
            9 +
            self.files.len() * (
                3 +
                20 +
                12 +
                5 + // Upper estimate
                31 +
                5 + // Upper estimate
                2
            ) +
            2
        );

        bytes.extend_from_slice(b"d5:filesd");
        
        for (info_hash, statistics) in self.files.iter(){
            bytes.extend_from_slice(b"20:");
            bytes.extend_from_slice(&info_hash.0);
            bytes.extend_from_slice(b"d8:completei");
            let _ = itoa::write(&mut bytes, statistics.complete);
            bytes.extend_from_slice(b"e10:downloadedi0e10:incompletei");
            let _ = itoa::write(&mut bytes, statistics.incomplete);
            bytes.extend_from_slice(b"ee");
        }

        bytes.extend_from_slice(b"ee");

        bytes
    }
}


#[derive(Debug, Clone, Serialize)]
pub struct FailureResponse {
    pub failure_reason: String,
}


impl FailureResponse {
    fn to_bytes(&self) -> Vec<u8> {
        let reason_bytes = self.failure_reason.as_bytes();

        let mut bytes = Vec::with_capacity(
            18 +
            3 + // Upper estimate
            1 +
            reason_bytes.len() +
            1
        ); 

        bytes.extend_from_slice(b"d14:failure_reason");
        let _ = itoa::write(&mut bytes, reason_bytes.len());
        bytes.push(b':');
        bytes.extend_from_slice(reason_bytes);
        bytes.push(b'e');

        bytes
    }
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
        match self {
            Response::Announce(r) => r.to_bytes(),
            Response::Failure(r) => r.to_bytes(),
            Response::Scrape(r) => r.to_bytes(),
        }
    }

    pub fn from_bytes(bytes: &[u8]) -> anyhow::Result<Self> {
        unimplemented!()
    }
}


#[cfg(test)]
impl quickcheck::Arbitrary for ResponsePeer<Ipv4Addr> {
    fn arbitrary<G: quickcheck::Gen>(g: &mut G) -> Self {
        Self {
            ip_address: Ipv4Addr::arbitrary(g),
            port: u16::arbitrary(g)
        }
    }
}


#[cfg(test)]
impl quickcheck::Arbitrary for ResponsePeer<Ipv6Addr> {
    fn arbitrary<G: quickcheck::Gen>(g: &mut G) -> Self {
        Self {
            ip_address: Ipv6Addr::arbitrary(g),
            port: u16::arbitrary(g)
        }
    }
}


#[cfg(test)]
impl quickcheck::Arbitrary for ResponsePeerListV4 {
    fn arbitrary<G: quickcheck::Gen>(g: &mut G) -> Self {
        Self(Vec::arbitrary(g))
    }
}


#[cfg(test)]
impl quickcheck::Arbitrary for ResponsePeerListV6 {
    fn arbitrary<G: quickcheck::Gen>(g: &mut G) -> Self {
        Self(Vec::arbitrary(g))
    }
}


#[cfg(test)]
impl quickcheck::Arbitrary for ScrapeStatistics {
    fn arbitrary<G: quickcheck::Gen>(g: &mut G) -> Self {
        Self {
            complete: usize::arbitrary(g),
            incomplete: usize::arbitrary(g),
            downloaded: 0,
        }
    }
}


#[cfg(test)]
impl quickcheck::Arbitrary for AnnounceResponse {
    fn arbitrary<G: quickcheck::Gen>(g: &mut G) -> Self {
        Self {
            announce_interval: usize::arbitrary(g),
            complete: usize::arbitrary(g),
            incomplete: usize::arbitrary(g),
            peers: ResponsePeerListV4::arbitrary(g),
            peers6: ResponsePeerListV6::arbitrary(g),
        }
    }
}


#[cfg(test)]
impl quickcheck::Arbitrary for ScrapeResponse {
    fn arbitrary<G: quickcheck::Gen>(g: &mut G) -> Self {
        Self {
            files: BTreeMap::arbitrary(g),
        }
    }
}


#[cfg(test)]
impl quickcheck::Arbitrary for FailureResponse {
    fn arbitrary<G: quickcheck::Gen>(g: &mut G) -> Self {
        Self {
            failure_reason: String::arbitrary(g),
        }
    }
}


#[cfg(test)]
mod tests {
    use quickcheck_macros::*;

    use super::*;

    #[quickcheck]
    fn test_announce_response_to_bytes(response: AnnounceResponse) -> bool {
        let reference = bendy::serde::to_bytes(
            &Response::Announce(response.clone())
        ).unwrap();

        response.to_bytes() == reference
    }

    #[quickcheck]
    fn test_scrape_response_to_bytes(response: ScrapeResponse) -> bool {
        let reference = bendy::serde::to_bytes(
            &Response::Scrape(response.clone())
        ).unwrap();
        let hand_written = response.to_bytes();

        let success = hand_written == reference;

        if !success {
            println!("reference: {}", String::from_utf8_lossy(&reference));
            println!("hand_written: {}", String::from_utf8_lossy(&hand_written));
        }

        success
    }

    #[quickcheck]
    fn test_failure_response_to_bytes(response: FailureResponse) -> bool {
        let reference = bendy::serde::to_bytes(
            &Response::Failure(response.clone())
        ).unwrap();

        response.to_bytes() == reference
    }
}