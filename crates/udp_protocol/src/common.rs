use std::fmt::Debug;
use std::net::{Ipv4Addr, Ipv6Addr};

pub use aquatic_peer_id::{PeerClient, PeerId};
use zerocopy::network_endian::{I32, I64, U16, U32};
use zerocopy::{AsBytes, FromBytes, FromZeroes};

#[cfg(feature = "serde")]
use serde::{Deserialize, Serialize};
#[cfg(feature = "serde")]
use serde_with::{hex::Hex, serde_as, FromInto};

// This mess is necessary because #[cfg_attr] doesn't seem to work on struct fields
macro_rules! zerocopy_newtype {
    ($newtype_name:ident, $inner_type:tt, $derive_as:expr) => {
        cfg_if::cfg_if! {
            if #[cfg(feature = "serde")] {
                #[derive(PartialEq, Eq, Hash, Clone, Copy, Debug, AsBytes, FromBytes, FromZeroes)]
                #[repr(transparent)]
                #[serde_as]
                #[derive(Serialize, Deserialize)]
                #[serde(transparent)]
                pub struct $newtype_name(
                    #[serde_as(as = $derive_as)]
                    pub $inner_type
                );
            } else {
                #[derive(PartialEq, Eq, Hash, Clone, Copy, Debug, AsBytes, FromBytes, FromZeroes)]
                #[repr(transparent)]
                pub struct $newtype_name(
                    pub $inner_type
                );
            }
        }
    };
}

pub trait Ip: Clone + Copy + Debug + PartialEq + Eq + std::hash::Hash + AsBytes {}

zerocopy_newtype!(AnnounceInterval, I32, "FromInto<i32>");

zerocopy_newtype!(ConnectionId, I64, "FromInto<i64>");

zerocopy_newtype!(TransactionId, I32, "FromInto<i32>");

zerocopy_newtype!(NumberOfBytes, I64, "FromInto<i64>");

zerocopy_newtype!(NumberOfPeers, I32, "FromInto<i32>");

zerocopy_newtype!(NumberOfDownloads, I32, "FromInto<i32>");

zerocopy_newtype!(Port, U16, "FromInto<u16>");

zerocopy_newtype!(PeerKey, I32, "FromInto<i32>");

zerocopy_newtype!(InfoHash, [u8; 20], "Hex");

#[derive(PartialEq, Eq, Clone, Copy, Debug, Hash, AsBytes, FromBytes, FromZeroes)]
#[repr(C, packed)]
#[cfg_attr(feature = "serde", derive(Serialize, Deserialize))]
pub struct ResponsePeer<I: Ip> {
    pub ip_address: I,
    pub port: Port,
}

#[derive(PartialEq, Eq, Hash, Clone, Copy, Debug, AsBytes, FromBytes, FromZeroes)]
#[repr(transparent)]
#[cfg_attr(
    feature = "serde",
    derive(Serialize, Deserialize),
    serde(from = "Ipv4Addr", into = "Ipv4Addr")
)]
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

#[derive(PartialEq, Eq, Hash, Clone, Copy, Debug, AsBytes, FromBytes, FromZeroes)]
#[repr(transparent)]
#[cfg_attr(
    feature = "serde",
    derive(Serialize, Deserialize),
    serde(from = "Ipv6Addr", into = "Ipv6Addr")
)]
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
