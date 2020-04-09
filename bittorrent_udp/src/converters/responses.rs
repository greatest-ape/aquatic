use byteorder::{ReadBytesExt, WriteBytesExt, NetworkEndian};

use std::io;
use std::io::{Cursor, Write};
use std::net::{IpAddr, Ipv6Addr, Ipv4Addr};

use crate::types;


#[inline]
pub fn response_to_bytes(
    bytes: &mut impl Write,
    response: types::Response,
    ip_version: types::IpVersion
){
    match response {
        types::Response::Connect(r) => {
            bytes.write_i32::<NetworkEndian>(0).unwrap();
            bytes.write_i32::<NetworkEndian>(r.transaction_id.0).unwrap();
            bytes.write_i64::<NetworkEndian>(r.connection_id.0).unwrap();
        },

        types::Response::Announce(r) => {
            bytes.write_i32::<NetworkEndian>(1).unwrap();
            bytes.write_i32::<NetworkEndian>(r.transaction_id.0).unwrap();
            bytes.write_i32::<NetworkEndian>(r.announce_interval.0).unwrap();
            bytes.write_i32::<NetworkEndian>(r.leechers.0).unwrap();
            bytes.write_i32::<NetworkEndian>(r.seeders.0).unwrap();

            // Write peer IPs and ports. Silently ignore peers with wrong
            // IP version
            for peer in r.peers {
                let mut correct = false;

                match peer.ip_address {
                    IpAddr::V4(ip) => {
                        if let types::IpVersion::IPv4 = ip_version {
                            bytes.write_all(&ip.octets()).unwrap();

                            correct = true;
                        }
                    },
                    IpAddr::V6(ip) => {
                        if let types::IpVersion::IPv6 = ip_version {
                            bytes.write_all(&ip.octets()).unwrap();

                            correct = true;
                        }
                    }
                }

                if correct {
                    bytes.write_u16::<NetworkEndian>(peer.port.0).unwrap();
                }
            }
        },

        types::Response::Scrape(r) => {
            bytes.write_i32::<NetworkEndian>(2).unwrap();
            bytes.write_i32::<NetworkEndian>(r.transaction_id.0).unwrap();

            for torrent_stat in r.torrent_stats {
                bytes.write_i32::<NetworkEndian>(torrent_stat.seeders.0)
                    .unwrap();
                bytes.write_i32::<NetworkEndian>(torrent_stat.completed.0)
                    .unwrap();
                bytes.write_i32::<NetworkEndian>(torrent_stat.leechers.0)
                    .unwrap();
            }
        },

        types::Response::Error(r) => {
            bytes.write_i32::<NetworkEndian>(3).unwrap();
            bytes.write_i32::<NetworkEndian>(r.transaction_id.0).unwrap();

            bytes.write_all(r.message.as_bytes()).unwrap();
        },
    }
}


#[inline]
pub fn response_from_bytes(
    bytes: &[u8],
    ip_version: types::IpVersion,
) -> Result<types::Response, io::Error> {

    let mut bytes = io::Cursor::new(bytes);

    let action         = bytes.read_i32::<NetworkEndian>()?;
    let transaction_id = bytes.read_i32::<NetworkEndian>()?;

    match action {

        // Connect
        0 => {
            let connection_id =  bytes.read_i64::<NetworkEndian>()?;

            Ok(types::Response::Connect(types::ConnectResponse {
                connection_id:  types::ConnectionId(connection_id),
                transaction_id: types::TransactionId(transaction_id)
            }))
        },

        // Announce
        1 => {
            let announce_interval =  bytes.read_i32::<NetworkEndian>()?;
            let leechers =  bytes.read_i32::<NetworkEndian>()?;
            let seeders =  bytes.read_i32::<NetworkEndian>()?;

            let position = bytes.position() as usize;
            let inner = bytes.into_inner();

            let peers = if ip_version == types::IpVersion::IPv4 {
                inner[position..].chunks_exact(6).map(|chunk| {
                    let ip_address = IpAddr::V4(
                        Ipv4Addr::new(chunk[0], chunk[1], chunk[2], chunk[3])
                    );

                    let port = (&chunk[4..]).read_u16::<NetworkEndian>().unwrap();

                    types::ResponsePeer {
                        ip_address,
                        port: types::Port(port),
                    }
                }).collect()
            } else {
                inner[position..].chunks_exact(18).map(|chunk| {
                    let mut cursor: Cursor<&[u8]> = Cursor::new(&chunk[..]);

                    let ip_address = IpAddr::V6(Ipv6Addr::new(
                        cursor.read_u16::<NetworkEndian>().unwrap(),
                        cursor.read_u16::<NetworkEndian>().unwrap(),
                        cursor.read_u16::<NetworkEndian>().unwrap(),
                        cursor.read_u16::<NetworkEndian>().unwrap(),
                        cursor.read_u16::<NetworkEndian>().unwrap(),
                        cursor.read_u16::<NetworkEndian>().unwrap(),
                        cursor.read_u16::<NetworkEndian>().unwrap(),
                        cursor.read_u16::<NetworkEndian>().unwrap(),
                    ));

                    let port = cursor.read_u16::<NetworkEndian>().unwrap();

                    types::ResponsePeer {
                        ip_address,
                        port: types::Port(port),
                    }
                }).collect()
            };

            Ok(types::Response::Announce(types::AnnounceResponse {
                transaction_id: types::TransactionId(transaction_id),
                announce_interval: types::AnnounceInterval(announce_interval),
                leechers: types::NumberOfPeers(leechers),
                seeders: types::NumberOfPeers(seeders),
                peers
            }))

        },

        // Scrape
        2 => {
            let position = bytes.position() as usize;
            let inner = bytes.into_inner();

            let stats = inner[position..].chunks_exact(12).map(|chunk| {
                let seeders = (&chunk[0..4]).read_i32::<NetworkEndian>().unwrap();
                let downloads = (&chunk[4..8]).read_i32::<NetworkEndian>().unwrap();
                let leechers = (&chunk[8..12]).read_i32::<NetworkEndian>().unwrap();

                types::TorrentScrapeStatistics {
                    seeders: types::NumberOfPeers(seeders),
                    completed: types::NumberOfDownloads(downloads),
                    leechers:types::NumberOfPeers(leechers)
                }
            }).collect();

            Ok(types::Response::Scrape(types::ScrapeResponse {
                transaction_id: types::TransactionId(transaction_id),
                torrent_stats: stats
            }))
        },

        // Error
        3 => {
            let position = bytes.position() as usize;
            let inner = bytes.into_inner();

            Ok(types::Response::Error(types::ErrorResponse {
                transaction_id: types::TransactionId(transaction_id),
                message: String::from_utf8_lossy(&inner[position..]).into()
            }))
        },

        _ => {
            Ok(types::Response::Error(types::ErrorResponse {
                transaction_id: types::TransactionId(transaction_id),
                message: "Invalid action".to_string()
            }))
        }
    }
}