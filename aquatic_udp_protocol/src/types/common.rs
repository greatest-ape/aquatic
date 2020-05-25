use std::net::IpAddr;


#[derive(PartialEq, Eq, Hash, Clone, Copy, Debug)]
pub enum IpVersion {
    IPv4,
    IPv6
}

#[derive(PartialEq, Eq, Hash, Clone, Copy, Debug)]
pub struct AnnounceInterval (pub i32);


#[derive(PartialEq, Eq, Hash, Clone, Copy, Debug)]
pub struct InfoHash (pub [u8; 20]);


#[derive(PartialEq, Eq, Hash, Clone, Copy, Debug)]
pub struct ConnectionId (pub i64);

#[derive(PartialEq, Eq, Hash, Clone, Copy, Debug)]
pub struct TransactionId (pub i32);


#[derive(PartialEq, Eq, Hash, Clone, Copy, Debug)]
pub struct NumberOfBytes (pub i64);

#[derive(PartialEq, Eq, Hash, Clone, Copy, Debug)]
pub struct NumberOfPeers (pub i32);

#[derive(PartialEq, Eq, Hash, Clone, Copy, Debug)]
pub struct NumberOfDownloads (pub i32);


#[derive(PartialEq, Eq, Hash, Clone, Copy, Debug)]
pub struct Port (pub u16);

#[derive(PartialEq, Eq, Hash, Clone, Copy, Debug, PartialOrd, Ord)]
pub struct PeerId (pub [u8; 20]);

#[derive(PartialEq, Eq, Hash, Clone, Copy, Debug)]
pub struct PeerKey (pub u32);


#[derive(Hash, PartialEq, Eq, Clone, Debug)]
pub struct ResponsePeer {
    pub ip_address: IpAddr,
    pub port: Port,
}


#[cfg(test)]
impl quickcheck::Arbitrary for IpVersion {
    fn arbitrary<G: quickcheck::Gen>(g: &mut G) -> Self {
        if bool::arbitrary(g) {
            IpVersion::IPv4
        } else {
            IpVersion::IPv6
        }
    }
}


#[cfg(test)]
impl quickcheck::Arbitrary for InfoHash {
    fn arbitrary<G: quickcheck::Gen>(g: &mut G) -> Self {
        let mut bytes = [0u8; 20];

        for byte in bytes.iter_mut() {
            *byte = u8::arbitrary(g);
        }

        Self(bytes)
    }
}


#[cfg(test)]
impl quickcheck::Arbitrary for PeerId {
    fn arbitrary<G: quickcheck::Gen>(g: &mut G) -> Self {
        let mut bytes = [0u8; 20];

        for byte in bytes.iter_mut() {
            *byte = u8::arbitrary(g);
        }

        Self(bytes)
    }
}


#[cfg(test)]
impl quickcheck::Arbitrary for ResponsePeer {
    fn arbitrary<G: quickcheck::Gen>(g: &mut G) -> Self {
        Self {
            ip_address: ::std::net::IpAddr::arbitrary(g),
            port: Port(u16::arbitrary(g)),
        }
    }
}