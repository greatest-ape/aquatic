use std::fmt::Debug;
use std::net::{Ipv4Addr, Ipv6Addr};

pub trait Ip: Clone + Copy + Debug + PartialEq + Eq {}

impl Ip for Ipv4Addr {}
impl Ip for Ipv6Addr {}

#[derive(PartialEq, Eq, Hash, Clone, Copy, Debug)]
pub struct AnnounceInterval(pub i32);

#[derive(PartialEq, Eq, Hash, Clone, Copy, Debug)]
pub struct InfoHash(pub [u8; 20]);

#[derive(PartialEq, Eq, Hash, Clone, Copy, Debug)]
pub struct ConnectionId(pub i64);

#[derive(PartialEq, Eq, Hash, Clone, Copy, Debug)]
pub struct TransactionId(pub i32);

#[derive(PartialEq, Eq, Hash, Clone, Copy, Debug)]
pub struct NumberOfBytes(pub i64);

#[derive(PartialEq, Eq, Hash, Clone, Copy, Debug)]
pub struct NumberOfPeers(pub i32);

#[derive(PartialEq, Eq, Hash, Clone, Copy, Debug)]
pub struct NumberOfDownloads(pub i32);

#[derive(PartialEq, Eq, Hash, Clone, Copy, Debug)]
pub struct Port(pub u16);

#[derive(PartialEq, Eq, Hash, Clone, Copy, Debug, PartialOrd, Ord)]
pub struct PeerId(pub [u8; 20]);

#[derive(PartialEq, Eq, Hash, Clone, Copy, Debug)]
pub struct PeerKey(pub u32);

#[derive(PartialEq, Eq, Clone, Debug)]
pub struct ResponsePeer<I: Ip> {
    pub ip_address: I,
    pub port: Port,
}

#[cfg(test)]
impl quickcheck::Arbitrary for InfoHash {
    fn arbitrary(g: &mut quickcheck::Gen) -> Self {
        let mut bytes = [0u8; 20];

        for byte in bytes.iter_mut() {
            *byte = u8::arbitrary(g);
        }

        Self(bytes)
    }
}

#[cfg(test)]
impl quickcheck::Arbitrary for PeerId {
    fn arbitrary(g: &mut quickcheck::Gen) -> Self {
        let mut bytes = [0u8; 20];

        for byte in bytes.iter_mut() {
            *byte = u8::arbitrary(g);
        }

        Self(bytes)
    }
}

#[cfg(test)]
impl<I: Ip + quickcheck::Arbitrary> quickcheck::Arbitrary for ResponsePeer<I> {
    fn arbitrary(g: &mut quickcheck::Gen) -> Self {
        Self {
            ip_address: quickcheck::Arbitrary::arbitrary(g),
            port: Port(u16::arbitrary(g).into()),
        }
    }
}
