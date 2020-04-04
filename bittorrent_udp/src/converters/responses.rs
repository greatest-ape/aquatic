use byteorder::{ReadBytesExt, WriteBytesExt, NetworkEndian};

use std::io;
use std::io::Read;
use std::net::{IpAddr, Ipv6Addr, Ipv4Addr};

use crate::types;


pub fn response_to_bytes(
    response: &types::Response,
    ip_version: types::IpVersion
) -> Vec<u8> {
    let mut bytes = Vec::new();

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
            for peer in r.peers.iter(){
                let mut correct = false;

                match peer.ip_address {
                    IpAddr::V4(ip) => {
                        if let types::IpVersion::IPv4 = ip_version {
                            bytes.extend(&ip.octets());
                            correct = true;
                        }
                    },
                    IpAddr::V6(ip) => {
                        if let types::IpVersion::IPv6 = ip_version {
                            bytes.extend(&ip.octets());
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

            for torrent_stat in r.torrent_stats.iter(){
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

            bytes.extend(r.message.as_bytes().iter());
        },
    }

    bytes
}


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

            let mut peers = Vec::new();

            loop {
                let mut opt_ip_address = None;

                match ip_version {
                    types::IpVersion::IPv4 => {
                        let mut ip_bytes = [0; 4];

                        if bytes.read_exact(&mut ip_bytes).is_ok() {
                            opt_ip_address = Some(IpAddr::V4(Ipv4Addr::new(
                                ip_bytes[0],
                                ip_bytes[1],
                                ip_bytes[2],
                                ip_bytes[3],
                            )));
                        }
                    },
                    types::IpVersion::IPv6 => {
                        let mut ip_bytes = [0; 16];

                        if bytes.read_exact(&mut ip_bytes).is_ok() {
                            let mut ip_bytes_ref = &ip_bytes[..];

                            opt_ip_address = Some(IpAddr::V6(Ipv6Addr::new(
                                ip_bytes_ref.read_u16::<NetworkEndian>()?,
                                ip_bytes_ref.read_u16::<NetworkEndian>()?,
                                ip_bytes_ref.read_u16::<NetworkEndian>()?,
                                ip_bytes_ref.read_u16::<NetworkEndian>()?,
                                ip_bytes_ref.read_u16::<NetworkEndian>()?,
                                ip_bytes_ref.read_u16::<NetworkEndian>()?,
                                ip_bytes_ref.read_u16::<NetworkEndian>()?,
                                ip_bytes_ref.read_u16::<NetworkEndian>()?,
                            )));
                        }
                    },
                }
                if let Some(ip_address) = opt_ip_address {
                    if let Ok(port) = bytes.read_u16::<NetworkEndian>() {
                        peers.push(types::ResponsePeer {
                            ip_address,
                            port: types::Port(port),
                        });
                    }
                    else {
                        break;
                    }
                }
                else {
                    break;
                }
            }

            Ok(types::Response::Announce(types::AnnounceResponse {
                transaction_id:    types::TransactionId(transaction_id),
                announce_interval: types::AnnounceInterval(announce_interval),
                leechers:          types::NumberOfPeers(leechers),
                seeders:           types::NumberOfPeers(seeders),
                peers
            }))

        },

        // Scrape
        2 => {
            let mut stats = Vec::new();

            // TODO: transition to while let && when available
            loop {
                if let Ok(seeders) = bytes.read_i32::<NetworkEndian>() {
                    if let Ok(downloaded) = bytes.read_i32::<NetworkEndian>() {
                        if let Ok(leechers) = bytes.read_i32::<NetworkEndian>() {
                            stats.push(types::TorrentScrapeStatistics {
                                seeders:   types::NumberOfPeers(seeders),
                                completed: types::NumberOfDownloads(downloaded),
                                leechers:  types::NumberOfPeers(leechers)
                            });
                        }
                        else {
                            break;
                        }
                    }
                    else {
                        break;
                    }
                }
                else {
                    break;
                }
            }

            Ok(types::Response::Scrape(types::ScrapeResponse {
                transaction_id: types::TransactionId(transaction_id),
                torrent_stats:  stats
            }))
        },

        // Error
        3 => {
            let mut message_bytes = Vec::new();

            bytes.read_to_end(&mut message_bytes)?;

            let message = match String::from_utf8(message_bytes) {
                Ok(message) => message,
                Err(_)      => "".to_string()
            };

            Ok(types::Response::Error(types::ErrorResponse {
                transaction_id: types::TransactionId(transaction_id),
                message
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