use byteorder::{ReadBytesExt, WriteBytesExt, NetworkEndian};

use std::convert::TryInto;
use std::io::{self, Cursor, Read, Write};
use std::net::Ipv4Addr;

use crate::types::*;

use super::common::*;


const PROTOCOL_IDENTIFIER: i64 = 4_497_486_125_440;


#[inline]
pub fn request_to_bytes(
    bytes: &mut impl Write,
    request: Request
){
    match request {
        Request::Connect(r) => {
            bytes.write_i64::<NetworkEndian>(PROTOCOL_IDENTIFIER).unwrap();
            bytes.write_i32::<NetworkEndian>(0).unwrap();
            bytes.write_i32::<NetworkEndian>(r.transaction_id.0).unwrap();
        },

        Request::Announce(r) => {
            bytes.write_i64::<NetworkEndian>(r.connection_id.0).unwrap();
            bytes.write_i32::<NetworkEndian>(1).unwrap();
            bytes.write_i32::<NetworkEndian>(r.transaction_id.0).unwrap();

            bytes.write_all(&r.info_hash.0).unwrap();
            bytes.write_all(&r.peer_id.0).unwrap();

            bytes.write_i64::<NetworkEndian>(r.bytes_downloaded.0).unwrap();
            bytes.write_i64::<NetworkEndian>(r.bytes_left.0).unwrap();
            bytes.write_i64::<NetworkEndian>(r.bytes_uploaded.0).unwrap();

            bytes.write_i32::<NetworkEndian>(event_to_i32(r.event)).unwrap();

            bytes.write_all(&r.ip_address.map_or(
                [0; 4],
                |ip| ip.octets()
            )).unwrap();

            bytes.write_u32::<NetworkEndian>(r.key.0).unwrap();
            bytes.write_i32::<NetworkEndian>(r.peers_wanted.0).unwrap();
            bytes.write_u16::<NetworkEndian>(r.port.0).unwrap();
        },

        Request::Scrape(r) => {
            bytes.write_i64::<NetworkEndian>(r.connection_id.0).unwrap();
            bytes.write_i32::<NetworkEndian>(2).unwrap();
            bytes.write_i32::<NetworkEndian>(r.transaction_id.0).unwrap();

            for info_hash in r.info_hashes {
                bytes.write_all(&info_hash.0).unwrap();
            }
        }

        _ => () // Invalid requests should never happen
    }
}


#[inline]
pub fn request_from_bytes(
    bytes: &[u8],
    max_scrape_torrents: u8,
) -> Result<Request,io::Error> {
    let mut cursor = Cursor::new(bytes);

    let connection_id = cursor.read_i64::<NetworkEndian>()?;
    let action = cursor.read_i32::<NetworkEndian>()?;
    let transaction_id = cursor.read_i32::<NetworkEndian>()?;

    match action {
        // Connect
        0 => {
            if connection_id == PROTOCOL_IDENTIFIER {
                Ok((ConnectRequest {
                    transaction_id: TransactionId(transaction_id)
                }).into())
            } else {
                Ok(Request::Invalid(InvalidRequest {
                    transaction_id: TransactionId(transaction_id),
                    message:
                        "Please send protocol identifier in connect request"
                        .to_string()
                }))
            }
        },

        // Announce
        1 => {
            let mut info_hash = [0; 20];
            let mut peer_id = [0; 20];
            let mut ip = [0; 4];

            cursor.read_exact(&mut info_hash)?;
            cursor.read_exact(&mut peer_id)?;

            let bytes_downloaded = cursor.read_i64::<NetworkEndian>()?;
            let bytes_left = cursor.read_i64::<NetworkEndian>()?;
            let bytes_uploaded = cursor.read_i64::<NetworkEndian>()?;
            let event = cursor.read_i32::<NetworkEndian>()?;

            cursor.read_exact(&mut ip)?;

            let key = cursor.read_u32::<NetworkEndian>()?;
            let peers_wanted = cursor.read_i32::<NetworkEndian>()?;
            let port = cursor.read_u16::<NetworkEndian>()?;

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
                event: event_from_i32(event),
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

        _ => Ok(Request::Invalid(InvalidRequest {
            transaction_id: TransactionId(transaction_id),
            message: "Invalid action".to_string()
        }))
    }
}
