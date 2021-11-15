use zerocopy::{AsBytes, FromBytes, NetworkEndian, Unaligned, I32, I64, U16, U32};

#[derive(PartialEq, Eq, Hash, Clone, Copy, Debug)]
pub enum IpVersion {
    IPv4,
    IPv6,
}

#[derive(PartialEq, Eq, Hash, Clone, Copy, Debug, AsBytes, FromBytes, Unaligned)]
#[repr(transparent)]
pub struct AnnounceInterval(pub I32<NetworkEndian>);

#[derive(PartialEq, Eq, Hash, Clone, Copy, Debug, AsBytes, FromBytes, Unaligned)]
#[repr(transparent)]
pub struct InfoHash(pub [u8; 20]);

#[derive(PartialEq, Eq, Hash, Clone, Copy, Debug, AsBytes, FromBytes, Unaligned)]
#[repr(transparent)]
pub struct ConnectionId(pub I64<NetworkEndian>);

#[derive(PartialEq, Eq, Hash, Clone, Copy, Debug, AsBytes, FromBytes, Unaligned)]
#[repr(transparent)]
pub struct TransactionId(pub I32<NetworkEndian>);

#[derive(PartialEq, Eq, Hash, Clone, Copy, Debug, AsBytes, FromBytes, Unaligned)]
#[repr(transparent)]
pub struct NumberOfBytes(pub I64<NetworkEndian>);

#[derive(PartialEq, Eq, Hash, Clone, Copy, Debug, AsBytes, FromBytes, Unaligned)]
#[repr(transparent)]
pub struct NumberOfPeers(pub I32<NetworkEndian>);

#[derive(PartialEq, Eq, Hash, Clone, Copy, Debug, AsBytes, FromBytes, Unaligned)]
#[repr(transparent)]
pub struct NumberOfDownloads(pub I32<NetworkEndian>);

#[derive(PartialEq, Eq, Hash, Clone, Copy, Debug, AsBytes, FromBytes, Unaligned)]
#[repr(transparent)]
pub struct Port(pub U16<NetworkEndian>);

#[derive(
    PartialEq, Eq, Hash, Clone, Copy, Debug, PartialOrd, Ord, AsBytes, FromBytes, Unaligned,
)]
#[repr(transparent)]
pub struct PeerId(pub [u8; 20]);

#[derive(PartialEq, Eq, Hash, Clone, Copy, Debug, AsBytes, FromBytes, Unaligned)]
#[repr(transparent)]
pub struct PeerKey(pub U32<NetworkEndian>);

#[derive(Hash, PartialEq, Eq, Clone, Debug, AsBytes, FromBytes, Unaligned)]
#[repr(C)]
pub struct ResponsePeerIpv4 {
    pub ip_address: [u8; 4],
    pub port: Port,
}

#[derive(Hash, PartialEq, Eq, Clone, Debug, AsBytes, FromBytes, Unaligned)]
#[repr(C)]
pub struct ResponsePeerIpv6 {
    pub ip_address: [u8; 16],
    pub port: Port,
}

#[derive(Clone, Copy, PartialEq, Eq, Debug, AsBytes, FromBytes, Unaligned)]
#[repr(transparent)]
pub struct ConnectAction(I32<NetworkEndian>);

impl ConnectAction {
    pub fn new() -> Self {
        Self(0.into())
    }
}

#[derive(Clone, Copy, PartialEq, Eq, Debug, AsBytes, FromBytes, Unaligned)]
#[repr(transparent)]
pub struct AnnounceAction(I32<NetworkEndian>);

impl AnnounceAction {
    pub fn new() -> Self {
        Self(1.into())
    }
}

#[derive(Clone, Copy, PartialEq, Eq, Debug, AsBytes, FromBytes, Unaligned)]
#[repr(transparent)]
pub struct ScrapeAction(I32<NetworkEndian>);

impl ScrapeAction {
    pub fn new() -> Self {
        Self(2.into())
    }
}

#[derive(Clone, Copy, PartialEq, Eq, Debug, AsBytes, FromBytes, Unaligned)]
#[repr(transparent)]
pub struct ErrorAction(I32<NetworkEndian>);

impl ErrorAction {
    pub fn new() -> Self {
        Self(3.into())
    }
}

#[derive(Clone, Copy, PartialEq, Eq, Debug, AsBytes, FromBytes, Unaligned)]
#[repr(transparent)]
pub struct AnnounceIpv6Action(I32<NetworkEndian>);

impl AnnounceIpv6Action {
    pub fn new() -> Self {
        Self(4.into())
    }
}

#[cfg(test)]
impl quickcheck::Arbitrary for IpVersion {
    fn arbitrary(g: &mut quickcheck::Gen) -> Self {
        if bool::arbitrary(g) {
            IpVersion::IPv4
        } else {
            IpVersion::IPv6
        }
    }
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
impl quickcheck::Arbitrary for ResponsePeerIpv4 {
    fn arbitrary(g: &mut quickcheck::Gen) -> Self {
        Self {
            ip_address: ::std::net::Ipv4Addr::arbitrary(g).octets(),
            port: Port(u16::arbitrary(g).into()),
        }
    }
}

#[cfg(test)]
impl quickcheck::Arbitrary for ResponsePeerIpv6 {
    fn arbitrary(g: &mut quickcheck::Gen) -> Self {
        Self {
            ip_address: ::std::net::Ipv6Addr::arbitrary(g).octets(),
            port: Port(u16::arbitrary(g).into()),
        }
    }
}
