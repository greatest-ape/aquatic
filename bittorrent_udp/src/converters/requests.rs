use byteorder::{ReadBytesExt, WriteBytesExt, NetworkEndian};

use std::io;
use std::io::Read;
use std::net::Ipv4Addr;

use crate::types;

use super::common::*;


const MAGIC_NUMBER: i64 = 4_497_486_125_440;


pub fn request_to_bytes(request: &types::Request) -> Vec<u8> {
    let mut bytes = Vec::new();

    match request {
        types::Request::Connect(r) => {
            bytes.write_i64::<NetworkEndian>(MAGIC_NUMBER).unwrap();
            bytes.write_i32::<NetworkEndian>(0).unwrap();
            bytes.write_i32::<NetworkEndian>(r.transaction_id.0).unwrap();
        },

        types::Request::Announce(r) => {
            bytes.write_i64::<NetworkEndian>(r.connection_id.0).unwrap();
            bytes.write_i32::<NetworkEndian>(1).unwrap();
            bytes.write_i32::<NetworkEndian>(r.transaction_id.0).unwrap();

            bytes.extend(r.info_hash.0.iter());
            bytes.extend(r.peer_id.0.iter());

            bytes.write_i64::<NetworkEndian>(r.bytes_downloaded.0).unwrap();
            bytes.write_i64::<NetworkEndian>(r.bytes_left.0).unwrap();
            bytes.write_i64::<NetworkEndian>(r.bytes_uploaded.0).unwrap();

            bytes.write_i32::<NetworkEndian>(event_to_i32(r.event)).unwrap();

            bytes.extend(&r.ip_address.map_or([0; 4], |ip| ip.octets()));

            bytes.write_u32::<NetworkEndian>(0).unwrap(); // IP
            bytes.write_u32::<NetworkEndian>(r.key.0).unwrap();
            bytes.write_i32::<NetworkEndian>(r.peers_wanted.0).unwrap();
            bytes.write_u16::<NetworkEndian>(r.port.0).unwrap();
        },

        types::Request::Scrape(r) => {
            bytes.write_i64::<NetworkEndian>(r.connection_id.0).unwrap();
            bytes.write_i32::<NetworkEndian>(2).unwrap();
            bytes.write_i32::<NetworkEndian>(r.transaction_id.0).unwrap();

            for info_hash in &r.info_hashes {
                bytes.extend(info_hash.0.iter());
            }
        }

        _ => () // Invalid requests should never happen
    }

    bytes
}


pub fn request_from_bytes(
    bytes: &[u8],
    max_scrape_torrents: u8,
) -> Result<types::Request,io::Error> {

    let mut bytes = io::Cursor::new(bytes);

    let connection_id =  bytes.read_i64::<NetworkEndian>()?;
    let action =         bytes.read_i32::<NetworkEndian>()?;
    let transaction_id = bytes.read_i32::<NetworkEndian>()?;

    match action {
        // Connect
        0 => {
            if connection_id == MAGIC_NUMBER {
                Ok(types::Request::Connect(types::ConnectRequest {
                    transaction_id:types::TransactionId(transaction_id)
                }))
            }
            else {
                Ok(types::Request::Invalid(types::InvalidRequest {
                    transaction_id:types::TransactionId(transaction_id),
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

            bytes.read_exact(&mut info_hash)?;
            bytes.read_exact(&mut peer_id)?;

            let bytes_downloaded = bytes.read_i64::<NetworkEndian>()?;
            let bytes_left       = bytes.read_i64::<NetworkEndian>()?;
            let bytes_uploaded   = bytes.read_i64::<NetworkEndian>()?;
            let event            = bytes.read_i32::<NetworkEndian>()?;

            bytes.read_exact(&mut ip)?;

            let key              = bytes.read_u32::<NetworkEndian>()?;
            let peers_wanted     = bytes.read_i32::<NetworkEndian>()?;
            let port             = bytes.read_u16::<NetworkEndian>()?;

            let opt_ip = if ip == [0; 4] {
                None
            }
            else {
                Some(Ipv4Addr::new(ip[0], ip[1], ip[2], ip[3]))
            };

            Ok(types::Request::Announce(types::AnnounceRequest {
                connection_id:    types::ConnectionId(connection_id),
                transaction_id:   types::TransactionId(transaction_id),
                info_hash:        types::InfoHash(info_hash),
                peer_id:          types::PeerId(peer_id),
                bytes_downloaded: types::NumberOfBytes(bytes_downloaded),
                bytes_uploaded:   types::NumberOfBytes(bytes_uploaded),
                bytes_left:       types::NumberOfBytes(bytes_left),
                event:            event_from_i32(event),
                ip_address:       opt_ip,
                key:              types::PeerKey(key),
                peers_wanted:     types::NumberOfPeers(peers_wanted),
                port:             types::Port(port)
            }))
        },

        // Scrape
        2 => {
            let mut info_hashes = Vec::new();
            let mut info_hash = [0; 20];

            let mut i = 0;

            loop {
                if i > max_scrape_torrents {
                    return Ok(types::Request::Invalid(types::InvalidRequest {
                        transaction_id:   types::TransactionId(transaction_id),
                        message: format!(
                            "Too many torrents. Maximum is {}",
                            max_scrape_torrents
                        )
                    }));
                }

                if bytes.read_exact(&mut info_hash).is_err(){
                    break
                }

                info_hashes.push(types::InfoHash(info_hash));

                i += 1;
            }

            Ok(types::Request::Scrape(types::ScrapeRequest {
                connection_id:  types::ConnectionId(connection_id),
                transaction_id: types::TransactionId(transaction_id),
                info_hashes
            }))
        }

        _ => Ok(types::Request::Invalid(types::InvalidRequest {
            transaction_id: types::TransactionId(transaction_id),
            message: "Invalid action".to_string()
        }))
    }
}
