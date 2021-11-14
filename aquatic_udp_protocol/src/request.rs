use std::convert::TryInto;
use std::io::{self, Write};
use std::mem::size_of;

use zerocopy::{AsBytes, FromBytes, NetworkEndian, Unaligned, I32, I64};

use super::common::*;

const PROTOCOL_IDENTIFIER: i64 = 4_497_486_125_440;

#[derive(PartialEq, Eq, Hash, Clone, Copy, Debug)]
pub enum AnnounceEvent {
    Completed,
    Started,
    Stopped,
    None,
}

#[derive(PartialEq, Eq, Hash, Clone, Copy, Debug, AsBytes, FromBytes, Unaligned)]
#[repr(transparent)]
pub struct AnnounceEventContainer(I32<NetworkEndian>);

impl Into<AnnounceEvent> for AnnounceEventContainer {
    fn into(self) -> AnnounceEvent {
        match self.0.get() {
            1 => AnnounceEvent::Completed,
            2 => AnnounceEvent::Started,
            3 => AnnounceEvent::Stopped,
            _ => AnnounceEvent::None,
        }
    }
}

impl Into<AnnounceEventContainer> for AnnounceEvent {
    fn into(self) -> AnnounceEventContainer {
        let n = match self {
            AnnounceEvent::Completed => 1,
            AnnounceEvent::Started => 2,
            AnnounceEvent::Stopped => 3,
            AnnounceEvent::None => 0,
        };

        AnnounceEventContainer(n.into())
    }
}

#[derive(PartialEq, Eq, Clone, Debug, AsBytes, FromBytes, Unaligned)]
#[repr(C)]
pub struct ConnectRequest {
    pub action: ConnectAction,
    pub transaction_id: TransactionId,
}

#[derive(PartialEq, Eq, Clone, Debug, AsBytes, FromBytes, Unaligned)]
#[repr(C)]
pub struct AnnounceRequest {
    pub connection_id: ConnectionId,
    pub action: AnnounceAction,
    pub transaction_id: TransactionId,
    pub info_hash: InfoHash,
    pub peer_id: PeerId,
    pub bytes_downloaded: NumberOfBytes,
    pub bytes_left: NumberOfBytes,
    pub bytes_uploaded: NumberOfBytes,
    pub event: AnnounceEventContainer,
    pub ip_address: [u8; 4],
    pub key: PeerKey,
    pub peers_wanted: NumberOfPeers,
    pub port: Port,
}

#[derive(PartialEq, Eq, Clone, Debug, AsBytes, FromBytes, Unaligned)]
#[repr(C)]
pub struct ScrapeRequestFixed {
    pub connection_id: ConnectionId,
    pub action: ScrapeAction,
    pub transaction_id: TransactionId,
}

#[derive(PartialEq, Eq, Clone, Debug)]
pub struct ScrapeRequest {
    pub fixed: ScrapeRequestFixed,
    pub info_hashes: Vec<InfoHash>,
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

impl Request {
    pub fn write(self, bytes: &mut impl Write) -> Result<(), io::Error> {
        match self {
            Request::Connect(r) => {
                let protocol_identifier: I64<NetworkEndian> = PROTOCOL_IDENTIFIER.into();
                bytes.write(protocol_identifier.as_bytes())?;

                bytes.write(r.as_bytes())?;
            }
            Request::Announce(r) => {
                bytes.write(r.as_bytes())?;
            }
            Request::Scrape(r) => {
                bytes.write(r.fixed.as_bytes())?;

                for info_hash in r.info_hashes {
                    bytes.write_all(&info_hash.0)?;
                }
            }
        }

        Ok(())
    }

    pub fn from_bytes(mut bytes: &[u8], max_scrape_torrents: u8) -> Option<Self> {
        let first_8_bytes: I64<NetworkEndian> = FromBytes::read_from_prefix(bytes)?;
        let action: I32<NetworkEndian> = FromBytes::read_from_prefix(&bytes[8..])?;

        match action.get() {
            0 if first_8_bytes.get() == PROTOCOL_IDENTIFIER => {
                ConnectRequest::read_from(&bytes[8..]).map(|r| r.into())
            }
            1 => AnnounceRequest::read_from(bytes).map(|r| r.into()),
            2 => {
                let fixed = ScrapeRequestFixed::read_from_prefix(bytes)?;
                bytes = &bytes[size_of::<ScrapeRequestFixed>()..];

                let info_hashes = bytes
                    .chunks_exact(20)
                    .take(max_scrape_torrents as usize)
                    .map(|chunk| InfoHash(chunk.try_into().unwrap()))
                    .collect();

                Some((ScrapeRequest { fixed, info_hashes }).into())
            }
            _ => None,
        }
    }
}

#[cfg(test)]
mod tests {
    use quickcheck_macros::quickcheck;

    use super::*;

    impl quickcheck::Arbitrary for AnnounceEventContainer {
        fn arbitrary(g: &mut quickcheck::Gen) -> Self {
            let event = match (bool::arbitrary(g), bool::arbitrary(g)) {
                (false, false) => AnnounceEvent::Started,
                (true, false) => AnnounceEvent::Started,
                (false, true) => AnnounceEvent::Completed,
                (true, true) => AnnounceEvent::None,
            };

            event.into()
        }
    }

    impl quickcheck::Arbitrary for ConnectRequest {
        fn arbitrary(g: &mut quickcheck::Gen) -> Self {
            Self {
                action: ConnectAction::new(),
                transaction_id: TransactionId(i32::arbitrary(g).into()),
            }
        }
    }

    impl quickcheck::Arbitrary for AnnounceRequest {
        fn arbitrary(g: &mut quickcheck::Gen) -> Self {
            Self {
                connection_id: ConnectionId(i64::arbitrary(g).into()),
                action: AnnounceAction::new(),
                transaction_id: TransactionId(i32::arbitrary(g).into()),
                info_hash: InfoHash::arbitrary(g),
                peer_id: PeerId::arbitrary(g),
                bytes_downloaded: NumberOfBytes(i64::arbitrary(g).into()),
                bytes_uploaded: NumberOfBytes(i64::arbitrary(g).into()),
                bytes_left: NumberOfBytes(i64::arbitrary(g).into()),
                event: AnnounceEventContainer::arbitrary(g),
                ip_address: [0; 4],
                key: PeerKey(u32::arbitrary(g).into()),
                peers_wanted: NumberOfPeers(i32::arbitrary(g).into()),
                port: Port(u16::arbitrary(g).into()),
            }
        }
    }

    impl quickcheck::Arbitrary for ScrapeRequest {
        fn arbitrary(g: &mut quickcheck::Gen) -> Self {
            let info_hashes = (0..u8::arbitrary(g))
                .map(|_| InfoHash::arbitrary(g))
                .collect();

            Self {
                fixed: ScrapeRequestFixed {
                    connection_id: ConnectionId(i64::arbitrary(g).into()),
                    action: ScrapeAction::new(),
                    transaction_id: TransactionId(i32::arbitrary(g).into()),
                },
                info_hashes,
            }
        }
    }

    fn same_after_conversion(request: Request) -> bool {
        let mut buf = Vec::new();

        request.clone().write(&mut buf).unwrap();
        let r2 = Request::from_bytes(&buf[..], ::std::u8::MAX).unwrap();

        let success = request == r2;

        if !success {
            println!("before: {:#?}\nafter: {:#?}", request, r2);
        }

        success
    }

    #[quickcheck]
    fn test_connect_request_convert_identity(request: ConnectRequest) -> bool {
        same_after_conversion(request.into())
    }

    #[quickcheck]
    fn test_announce_request_convert_identity(request: AnnounceRequest) -> bool {
        same_after_conversion(request.into())
    }

    #[quickcheck]
    fn test_scrape_request_convert_identity(request: ScrapeRequest) -> bool {
        same_after_conversion(request.into())
    }
}
