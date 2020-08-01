use std::convert::TryInto;
use std::io::{self, Cursor, Read, Write};
use std::net::Ipv4Addr;

use byteorder::{ReadBytesExt, WriteBytesExt, NetworkEndian};

use super::common::*;


const PROTOCOL_IDENTIFIER: i64 = 4_497_486_125_440;


#[derive(PartialEq, Eq, Hash, Clone, Copy, Debug)]
pub enum AnnounceEvent {
    Started,
    Stopped,
    Completed,
    None
}


impl AnnounceEvent {
    #[inline]
    pub fn from_i32(i: i32) -> Self {
        match i {
            1 => Self::Completed,
            2 => Self::Started,
            3 => Self::Stopped,
            _ => Self::None
        }
    }

    #[inline]
    pub fn to_i32(&self) -> i32 {
        match self {
            AnnounceEvent::None => 0,
            AnnounceEvent::Completed => 1,
            AnnounceEvent::Started => 2,
            AnnounceEvent::Stopped => 3
        }
    }
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


#[derive(Debug)]
pub struct RequestParseError {
    pub transaction_id: Option<TransactionId>,
    pub message: Option<String>,
    pub error: Option<io::Error>,
}


impl RequestParseError {
    pub fn new(err: io::Error, transaction_id: i32) -> Self {
        Self {
            transaction_id: Some(TransactionId(transaction_id)),
            message: None,
            error: Some(err)
        }
    }
    pub fn io(err: io::Error) -> Self {
        Self {
            transaction_id: None,
            message: None,
            error: Some(err)
        }
    }
    pub fn text(transaction_id: i32, message: &str) -> Self {
        Self {
            transaction_id: Some(TransactionId(transaction_id)),
            message: Some(message.to_string()),
            error: None,
        }
    }
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
                bytes.write_i64::<NetworkEndian>(PROTOCOL_IDENTIFIER)?;
                bytes.write_i32::<NetworkEndian>(0)?;
                bytes.write_i32::<NetworkEndian>(r.transaction_id.0)?;
            },

            Request::Announce(r) => {
                bytes.write_i64::<NetworkEndian>(r.connection_id.0)?;
                bytes.write_i32::<NetworkEndian>(1)?;
                bytes.write_i32::<NetworkEndian>(r.transaction_id.0)?;

                bytes.write_all(&r.info_hash.0)?;
                bytes.write_all(&r.peer_id.0)?;

                bytes.write_i64::<NetworkEndian>(r.bytes_downloaded.0)?;
                bytes.write_i64::<NetworkEndian>(r.bytes_left.0)?;
                bytes.write_i64::<NetworkEndian>(r.bytes_uploaded.0)?;

                bytes.write_i32::<NetworkEndian>(r.event.to_i32())?;

                bytes.write_all(&r.ip_address.map_or(
                    [0; 4],
                    |ip| ip.octets()
                ))?;

                bytes.write_u32::<NetworkEndian>(r.key.0)?;
                bytes.write_i32::<NetworkEndian>(r.peers_wanted.0)?;
                bytes.write_u16::<NetworkEndian>(r.port.0)?;
            },

            Request::Scrape(r) => {
                bytes.write_i64::<NetworkEndian>(r.connection_id.0)?;
                bytes.write_i32::<NetworkEndian>(2)?;
                bytes.write_i32::<NetworkEndian>(r.transaction_id.0)?;

                for info_hash in r.info_hashes {
                    bytes.write_all(&info_hash.0)?;
                }
            }
        }

        Ok(())
    }

    pub fn from_bytes(
        bytes: &[u8],
        max_scrape_torrents: u8,
    ) -> Result<Self, RequestParseError> {
        let mut cursor = Cursor::new(bytes);

        let connection_id = cursor.read_i64::<NetworkEndian>()
            .map_err(RequestParseError::io)?;
        let action = cursor.read_i32::<NetworkEndian>()
            .map_err(RequestParseError::io)?;
        let transaction_id = cursor.read_i32::<NetworkEndian>()
            .map_err(RequestParseError::io)?;

        match action {
            // Connect
            0 => {
                if connection_id == PROTOCOL_IDENTIFIER {
                    Ok((ConnectRequest {
                        transaction_id: TransactionId(transaction_id)
                    }).into())
                } else {
                    Err(RequestParseError::text(
                        transaction_id,
                        "Protocol identifier missing"
                    ))
                }
            },

            // Announce
            1 => {
                let mut info_hash = [0; 20];
                let mut peer_id = [0; 20];
                let mut ip = [0; 4];

                cursor.read_exact(&mut info_hash)
                    .map_err(|err| RequestParseError::new(err, transaction_id))?;
                cursor.read_exact(&mut peer_id)
                    .map_err(|err| RequestParseError::new(err, transaction_id))?;

                let bytes_downloaded = cursor.read_i64::<NetworkEndian>()
                    .map_err(|err| RequestParseError::new(err, transaction_id))?;
                let bytes_left = cursor.read_i64::<NetworkEndian>()
                    .map_err(|err| RequestParseError::new(err, transaction_id))?;
                let bytes_uploaded = cursor.read_i64::<NetworkEndian>()
                    .map_err(|err| RequestParseError::new(err, transaction_id))?;
                let event = cursor.read_i32::<NetworkEndian>()
                    .map_err(|err| RequestParseError::new(err, transaction_id))?;

                cursor.read_exact(&mut ip)
                    .map_err(|err| RequestParseError::new(err, transaction_id))?;

                let key = cursor.read_u32::<NetworkEndian>()
                    .map_err(|err| RequestParseError::new(err, transaction_id))?;
                let peers_wanted = cursor.read_i32::<NetworkEndian>()
                    .map_err(|err| RequestParseError::new(err, transaction_id))?;
                let port = cursor.read_u16::<NetworkEndian>()
                    .map_err(|err| RequestParseError::new(err, transaction_id))?;

                let opt_ip = if ip == [0; 4] {
                    None
                } else {
                    Some(Ipv4Addr::from(ip))
                };

                Ok((AnnounceRequest {
                    connection_id: ConnectionId(connection_id),
                    transaction_id: TransactionId(transaction_id),
                    info_hash: InfoHash(info_hash),
                    peer_id: PeerId(peer_id),
                    bytes_downloaded: NumberOfBytes(bytes_downloaded),
                    bytes_uploaded: NumberOfBytes(bytes_uploaded),
                    bytes_left: NumberOfBytes(bytes_left),
                    event: AnnounceEvent::from_i32(event),
                    ip_address: opt_ip,
                    key: PeerKey(key),
                    peers_wanted: NumberOfPeers(peers_wanted),
                    port: Port(port)
                }).into())
            },

            // Scrape
            2 => {
                let position = cursor.position() as usize;
                let inner = cursor.into_inner();

                let info_hashes = (&inner[position..]).chunks_exact(20)
                    .take(max_scrape_torrents as usize)
                    .map(|chunk| InfoHash(chunk.try_into().unwrap()))
                    .collect();

                Ok((ScrapeRequest {
                    connection_id: ConnectionId(connection_id),
                    transaction_id: TransactionId(transaction_id),
                    info_hashes
                }).into())
            }

            _ => Err(RequestParseError::text(transaction_id, "Invalid action"))
        }
    }
}


#[cfg(test)]
mod tests {
    use quickcheck_macros::quickcheck;

    use super::*;

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

    impl quickcheck::Arbitrary for ConnectRequest {
        fn arbitrary<G: quickcheck::Gen>(g: &mut G) -> Self {
            Self {
                transaction_id: TransactionId(i32::arbitrary(g)),
            }
        }
    }

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
    fn test_connect_request_convert_identity(
        request: ConnectRequest
    ) -> bool {
        same_after_conversion(request.into())
    }   

    #[quickcheck]
    fn test_announce_request_convert_identity(
        request: AnnounceRequest
    ) -> bool {
        same_after_conversion(request.into())
    }   

    #[quickcheck]
    fn test_scrape_request_convert_identity(
        request: ScrapeRequest
    ) -> bool {
        same_after_conversion(request.into())
    }   
}