use std::fmt::Debug;
use std::net::{Ipv4Addr, Ipv6Addr};
use std::num::NonZeroU16;

pub use aquatic_peer_id::{PeerClient, PeerId};
use zerocopy::network_endian::{I32, I64, U16, U32};
use zerocopy::{AsBytes, FromBytes, FromZeroes};

pub trait Ip: Clone + Copy + Debug + PartialEq + Eq + std::hash::Hash + AsBytes {}

#[derive(PartialEq, Eq, Hash, Clone, Copy, Debug, AsBytes, FromBytes, FromZeroes)]
#[repr(transparent)]
pub struct AnnounceInterval(pub I32);

impl AnnounceInterval {
    pub fn new(v: i32) -> Self {
        Self(I32::new(v))
    }
}

impl Ord for AnnounceInterval {
    fn cmp(&self, other: &Self) -> std::cmp::Ordering {
        self.0.get().cmp(&other.0.get())
    }
}

impl PartialOrd for AnnounceInterval {
    fn partial_cmp(&self, other: &Self) -> Option<std::cmp::Ordering> {
        Some(self.cmp(&other))
    }
}

#[derive(
    PartialEq, Eq, PartialOrd, Ord, Hash, Clone, Copy, Debug, AsBytes, FromBytes, FromZeroes,
)]
#[repr(transparent)]
pub struct InfoHash(pub [u8; 20]);

#[derive(PartialEq, Eq, Hash, Clone, Copy, Debug, AsBytes, FromBytes, FromZeroes)]
#[repr(transparent)]
pub struct ConnectionId(pub I64);

impl ConnectionId {
    pub fn new(v: i64) -> Self {
        Self(I64::new(v))
    }
}

impl Ord for ConnectionId {
    fn cmp(&self, other: &Self) -> std::cmp::Ordering {
        self.0.get().cmp(&other.0.get())
    }
}

impl PartialOrd for ConnectionId {
    fn partial_cmp(&self, other: &Self) -> Option<std::cmp::Ordering> {
        Some(self.cmp(&other))
    }
}

#[derive(PartialEq, Eq, Hash, Clone, Copy, Debug, AsBytes, FromBytes, FromZeroes)]
#[repr(transparent)]
pub struct TransactionId(pub I32);

impl TransactionId {
    pub fn new(v: i32) -> Self {
        Self(I32::new(v))
    }
}

impl Ord for TransactionId {
    fn cmp(&self, other: &Self) -> std::cmp::Ordering {
        self.0.get().cmp(&other.0.get())
    }
}

impl PartialOrd for TransactionId {
    fn partial_cmp(&self, other: &Self) -> Option<std::cmp::Ordering> {
        Some(self.cmp(&other))
    }
}

#[derive(PartialEq, Eq, Hash, Clone, Copy, Debug, AsBytes, FromBytes, FromZeroes)]
#[repr(transparent)]
pub struct NumberOfBytes(pub I64);

impl NumberOfBytes {
    pub fn new(v: i64) -> Self {
        Self(I64::new(v))
    }
}

impl Ord for NumberOfBytes {
    fn cmp(&self, other: &Self) -> std::cmp::Ordering {
        self.0.get().cmp(&other.0.get())
    }
}

impl PartialOrd for NumberOfBytes {
    fn partial_cmp(&self, other: &Self) -> Option<std::cmp::Ordering> {
        Some(self.cmp(&other))
    }
}

#[derive(PartialEq, Eq, Hash, Clone, Copy, Debug, AsBytes, FromBytes, FromZeroes)]
#[repr(transparent)]
pub struct NumberOfPeers(pub I32);

impl NumberOfPeers {
    pub fn new(v: i32) -> Self {
        Self(I32::new(v))
    }
}

impl Ord for NumberOfPeers {
    fn cmp(&self, other: &Self) -> std::cmp::Ordering {
        self.0.get().cmp(&other.0.get())
    }
}

impl PartialOrd for NumberOfPeers {
    fn partial_cmp(&self, other: &Self) -> Option<std::cmp::Ordering> {
        Some(self.cmp(&other))
    }
}

#[derive(PartialEq, Eq, Hash, Clone, Copy, Debug, AsBytes, FromBytes, FromZeroes)]
#[repr(transparent)]
pub struct NumberOfDownloads(pub I32);

impl NumberOfDownloads {
    pub fn new(v: i32) -> Self {
        Self(I32::new(v))
    }
}

impl Ord for NumberOfDownloads {
    fn cmp(&self, other: &Self) -> std::cmp::Ordering {
        self.0.get().cmp(&other.0.get())
    }
}

impl PartialOrd for NumberOfDownloads {
    fn partial_cmp(&self, other: &Self) -> Option<std::cmp::Ordering> {
        Some(self.cmp(&other))
    }
}

#[derive(PartialEq, Eq, Hash, Clone, Copy, Debug, AsBytes, FromBytes, FromZeroes)]
#[repr(transparent)]
pub struct Port(pub U16);

impl Port {
    pub fn new(v: NonZeroU16) -> Self {
        Self(U16::new(v.into()))
    }
}

impl Ord for Port {
    fn cmp(&self, other: &Self) -> std::cmp::Ordering {
        self.0.get().cmp(&other.0.get())
    }
}

impl PartialOrd for Port {
    fn partial_cmp(&self, other: &Self) -> Option<std::cmp::Ordering> {
        Some(self.cmp(&other))
    }
}

#[derive(PartialEq, Eq, Hash, Clone, Copy, Debug, AsBytes, FromBytes, FromZeroes)]
#[repr(transparent)]
pub struct PeerKey(pub I32);

impl PeerKey {
    pub fn new(v: i32) -> Self {
        Self(I32::new(v))
    }
}

impl Ord for PeerKey {
    fn cmp(&self, other: &Self) -> std::cmp::Ordering {
        self.0.get().cmp(&other.0.get())
    }
}

impl PartialOrd for PeerKey {
    fn partial_cmp(&self, other: &Self) -> Option<std::cmp::Ordering> {
        Some(self.cmp(&other))
    }
}

#[derive(
    PartialEq, Eq, PartialOrd, Ord, Clone, Copy, Debug, Hash, AsBytes, FromBytes, FromZeroes,
)]
#[repr(C, packed)]
pub struct ResponsePeer<I: Ip> {
    pub ip_address: I,
    pub port: Port,
}

#[derive(
    PartialEq, Eq, PartialOrd, Ord, Hash, Clone, Copy, Debug, AsBytes, FromBytes, FromZeroes,
)]
#[repr(transparent)]
pub struct Ipv4AddrBytes(pub [u8; 4]);

impl Ip for Ipv4AddrBytes {}

impl From<Ipv4AddrBytes> for Ipv4Addr {
    fn from(val: Ipv4AddrBytes) -> Self {
        Ipv4Addr::from(val.0)
    }
}

impl From<Ipv4Addr> for Ipv4AddrBytes {
    fn from(val: Ipv4Addr) -> Self {
        Ipv4AddrBytes(val.octets())
    }
}

#[derive(
    PartialEq, Eq, PartialOrd, Ord, Hash, Clone, Copy, Debug, AsBytes, FromBytes, FromZeroes,
)]
#[repr(transparent)]
pub struct Ipv6AddrBytes(pub [u8; 16]);

impl Ip for Ipv6AddrBytes {}

impl From<Ipv6AddrBytes> for Ipv6Addr {
    fn from(val: Ipv6AddrBytes) -> Self {
        Ipv6Addr::from(val.0)
    }
}

impl From<Ipv6Addr> for Ipv6AddrBytes {
    fn from(val: Ipv6Addr) -> Self {
        Ipv6AddrBytes(val.octets())
    }
}

pub fn read_i32_ne(bytes: &mut impl ::std::io::Read) -> ::std::io::Result<I32> {
    let mut tmp = [0u8; 4];

    bytes.read_exact(&mut tmp)?;

    Ok(I32::from_bytes(tmp))
}

pub fn read_i64_ne(bytes: &mut impl ::std::io::Read) -> ::std::io::Result<I64> {
    let mut tmp = [0u8; 8];

    bytes.read_exact(&mut tmp)?;

    Ok(I64::from_bytes(tmp))
}

pub fn read_u16_ne(bytes: &mut impl ::std::io::Read) -> ::std::io::Result<U16> {
    let mut tmp = [0u8; 2];

    bytes.read_exact(&mut tmp)?;

    Ok(U16::from_bytes(tmp))
}

pub fn read_u32_ne(bytes: &mut impl ::std::io::Read) -> ::std::io::Result<U32> {
    let mut tmp = [0u8; 4];

    bytes.read_exact(&mut tmp)?;

    Ok(U32::from_bytes(tmp))
}

pub fn invalid_data() -> ::std::io::Error {
    ::std::io::Error::new(::std::io::ErrorKind::InvalidData, "invalid data")
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
impl<I: Ip + quickcheck::Arbitrary> quickcheck::Arbitrary for ResponsePeer<I> {
    fn arbitrary(g: &mut quickcheck::Gen) -> Self {
        Self {
            ip_address: quickcheck::Arbitrary::arbitrary(g),
            port: Port(u16::arbitrary(g).into()),
        }
    }
}
