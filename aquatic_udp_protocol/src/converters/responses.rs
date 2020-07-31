use std::convert::TryInto;
use std::io::{self, Cursor, Write};
use std::net::{IpAddr, Ipv6Addr, Ipv4Addr};

use byteorder::{ReadBytesExt, WriteBytesExt, NetworkEndian};

use crate::types::*;


/// Returning IPv6 peers doesn't really work with UDP. It is not supported by
/// https://libtorrent.org/udp_tracker_protocol.html. There is a suggestion in
/// https://web.archive.org/web/20170503181830/http://opentracker.blog.h3q.com/2007/12/28/the-ipv6-situation/
/// of using action number 4 and returning IPv6 octets just like for IPv4
/// addresses. Clients seem not to support it very well, but due to a lack of
/// alternative solutions, it is implemented here.
#[inline]
pub fn response_to_bytes(
    bytes: &mut impl Write,
    response: Response,
    ip_version: IpVersion
) -> Result<(), io::Error> {
    match response {
        Response::Connect(r) => {
            bytes.write_i32::<NetworkEndian>(0)?;
            bytes.write_i32::<NetworkEndian>(r.transaction_id.0)?;
            bytes.write_i64::<NetworkEndian>(r.connection_id.0)?;
        },
        Response::Announce(r) => {
            if ip_version == IpVersion::IPv4 {
                bytes.write_i32::<NetworkEndian>(1)?;
                bytes.write_i32::<NetworkEndian>(r.transaction_id.0)?;
                bytes.write_i32::<NetworkEndian>(r.announce_interval.0)?;
                bytes.write_i32::<NetworkEndian>(r.leechers.0)?;
                bytes.write_i32::<NetworkEndian>(r.seeders.0)?;

                // Silently ignore peers with wrong IP version
                for peer in r.peers {
                    if let IpAddr::V4(ip) = peer.ip_address {
                        bytes.write_all(&ip.octets())?;
                        bytes.write_u16::<NetworkEndian>(peer.port.0)?;
                    }
                }
            } else {
                bytes.write_i32::<NetworkEndian>(4)?;
                bytes.write_i32::<NetworkEndian>(r.transaction_id.0)?;
                bytes.write_i32::<NetworkEndian>(r.announce_interval.0)?;
                bytes.write_i32::<NetworkEndian>(r.leechers.0)?;
                bytes.write_i32::<NetworkEndian>(r.seeders.0)?;

                // Silently ignore peers with wrong IP version
                for peer in r.peers {
                    if let IpAddr::V6(ip) = peer.ip_address {
                        bytes.write_all(&ip.octets())?;
                        bytes.write_u16::<NetworkEndian>(peer.port.0)?;
                    }
                }
            }
        },
        Response::Scrape(r) => {
            bytes.write_i32::<NetworkEndian>(2)?;
            bytes.write_i32::<NetworkEndian>(r.transaction_id.0)?;

            for torrent_stat in r.torrent_stats {
                bytes.write_i32::<NetworkEndian>(torrent_stat.seeders.0)?;
                bytes.write_i32::<NetworkEndian>(torrent_stat.completed.0)?;
                bytes.write_i32::<NetworkEndian>(torrent_stat.leechers.0)?;
            }
        },
        Response::Error(r) => {
            bytes.write_i32::<NetworkEndian>(3)?;
            bytes.write_i32::<NetworkEndian>(r.transaction_id.0)?;

            bytes.write_all(r.message.as_bytes())?;
        },
    }

    Ok(())
}


#[inline]
pub fn response_from_bytes(
    bytes: &[u8],
    ip_version: IpVersion,
) -> Result<Response, io::Error> {
    let mut cursor = Cursor::new(bytes);

    let action = cursor.read_i32::<NetworkEndian>()?;
    let transaction_id = cursor.read_i32::<NetworkEndian>()?;

    match action {
        // Connect
        0 => {
            let connection_id = cursor.read_i64::<NetworkEndian>()?;

            Ok((ConnectResponse {
                connection_id: ConnectionId(connection_id),
                transaction_id: TransactionId(transaction_id)
            }).into())
        },
        // Announce
        1 => {
            let announce_interval = cursor.read_i32::<NetworkEndian>()?;
            let leechers = cursor.read_i32::<NetworkEndian>()?;
            let seeders = cursor.read_i32::<NetworkEndian>()?;

            let position = cursor.position() as usize;
            let inner = cursor.into_inner();

            let peers = if ip_version == IpVersion::IPv4 {
                inner[position..].chunks_exact(6).map(|chunk| {
                    let ip_bytes: [u8; 4] = (&chunk[..4]).try_into().unwrap();
                    let ip_address = IpAddr::V4(Ipv4Addr::from(ip_bytes));
                    let port = (&chunk[4..]).read_u16::<NetworkEndian>().unwrap();

                    ResponsePeer {
                        ip_address,
                        port: Port(port),
                    }
                }).collect()
            } else {
                inner[position..].chunks_exact(18).map(|chunk| {
                    let ip_bytes: [u8; 16] = (&chunk[..16]).try_into().unwrap();
                    let ip_address = IpAddr::V6(Ipv6Addr::from(ip_bytes));
                    let port = (&chunk[16..]).read_u16::<NetworkEndian>().unwrap();

                    ResponsePeer {
                        ip_address,
                        port: Port(port),
                    }
                }).collect()
            };

            Ok((AnnounceResponse {
                transaction_id: TransactionId(transaction_id),
                announce_interval: AnnounceInterval(announce_interval),
                leechers: NumberOfPeers(leechers),
                seeders: NumberOfPeers(seeders),
                peers
            }).into())

        },
        // Scrape
        2 => {
            let position = cursor.position() as usize;
            let inner = cursor.into_inner();

            let stats = inner[position..].chunks_exact(12).map(|chunk| {
                let mut cursor: Cursor<&[u8]> = Cursor::new(&chunk[..]);

                let seeders = cursor.read_i32::<NetworkEndian>().unwrap();
                let downloads = cursor.read_i32::<NetworkEndian>().unwrap();
                let leechers = cursor.read_i32::<NetworkEndian>().unwrap();

                TorrentScrapeStatistics {
                    seeders: NumberOfPeers(seeders),
                    completed: NumberOfDownloads(downloads),
                    leechers:NumberOfPeers(leechers)
                }
            }).collect();

            Ok((ScrapeResponse {
                transaction_id: TransactionId(transaction_id),
                torrent_stats: stats
            }).into())
        },
        // Error
        3 => {
            let position = cursor.position() as usize;
            let inner = cursor.into_inner();

            Ok((ErrorResponse {
                transaction_id: TransactionId(transaction_id),
                message: String::from_utf8_lossy(&inner[position..]).into()
            }).into())
        },
        _ => {
            Ok((ErrorResponse {
                transaction_id: TransactionId(transaction_id),
                message: "Invalid action".to_string()
            }).into())
        }
    }
}


#[cfg(test)]
mod tests {
    use super::*;

    fn same_after_conversion(
        response: Response,
        ip_version: IpVersion
    ) -> bool {
        let mut buf = Vec::new();

        response_to_bytes(&mut buf, response.clone(), ip_version).unwrap();
        let r2 = response_from_bytes(&buf[..], ip_version).unwrap();

        let success = response == r2;

        if !success {
            println!("before: {:#?}\nafter: {:#?}", response, r2);
        }

        success
    }

    #[quickcheck]
    fn test_connect_response_convert_identity(
        response: ConnectResponse
    ) -> bool {
        same_after_conversion(response.into(), IpVersion::IPv4)
    }   

    #[quickcheck]
    fn test_announce_response_convert_identity(
        data: (AnnounceResponse, IpVersion)
    ) -> bool {
        let mut r = data.0;

        if data.1 == IpVersion::IPv4 {
            r.peers.retain(|peer| peer.ip_address.is_ipv4());
        } else {
            r.peers.retain(|peer| peer.ip_address.is_ipv6());
        }

        same_after_conversion(r.into(), data.1)
    }   

    #[quickcheck]
    fn test_scrape_response_convert_identity(
        response: ScrapeResponse
    ) -> bool {
        same_after_conversion(response.into(), IpVersion::IPv4)
    }   
}