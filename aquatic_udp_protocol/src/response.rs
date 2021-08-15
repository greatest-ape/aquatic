use std::convert::TryInto;
use std::io::{self, Cursor, Write};
use std::net::{IpAddr, Ipv4Addr, Ipv6Addr};

use byteorder::{NetworkEndian, ReadBytesExt, WriteBytesExt};

use super::common::*;

#[derive(PartialEq, Eq, Debug, Copy, Clone)]
pub struct TorrentScrapeStatistics {
    pub seeders: NumberOfPeers,
    pub completed: NumberOfDownloads,
    pub leechers: NumberOfPeers,
}

#[derive(PartialEq, Eq, Clone, Debug)]
pub struct ConnectResponse {
    pub connection_id: ConnectionId,
    pub transaction_id: TransactionId,
}

#[derive(PartialEq, Eq, Clone, Debug)]
pub struct AnnounceResponse {
    pub transaction_id: TransactionId,
    pub announce_interval: AnnounceInterval,
    pub leechers: NumberOfPeers,
    pub seeders: NumberOfPeers,
    pub peers: Vec<ResponsePeer>,
}

#[derive(PartialEq, Eq, Clone, Debug)]
pub struct ScrapeResponse {
    pub transaction_id: TransactionId,
    pub torrent_stats: Vec<TorrentScrapeStatistics>,
}

#[derive(PartialEq, Eq, Clone, Debug)]
pub struct ErrorResponse {
    pub transaction_id: TransactionId,
    pub message: String,
}

#[derive(PartialEq, Eq, Clone, Debug)]
pub enum Response {
    Connect(ConnectResponse),
    Announce(AnnounceResponse),
    Scrape(ScrapeResponse),
    Error(ErrorResponse),
}

impl From<ConnectResponse> for Response {
    fn from(r: ConnectResponse) -> Self {
        Self::Connect(r)
    }
}

impl From<AnnounceResponse> for Response {
    fn from(r: AnnounceResponse) -> Self {
        Self::Announce(r)
    }
}

impl From<ScrapeResponse> for Response {
    fn from(r: ScrapeResponse) -> Self {
        Self::Scrape(r)
    }
}

impl From<ErrorResponse> for Response {
    fn from(r: ErrorResponse) -> Self {
        Self::Error(r)
    }
}

impl Response {
    /// Returning IPv6 peers doesn't really work with UDP. It is not supported
    /// by https://libtorrent.org/udp_tracker_protocol.html. There is a
    /// suggestion in https://web.archive.org/web/20170503181830/http://opentracker.blog.h3q.com/2007/12/28/the-ipv6-situation/
    /// of using action number 4 and returning IPv6 octets just like for IPv4
    /// addresses. Clients seem not to support it very well, but due to a lack
    /// of alternative solutions, it is implemented here.
    #[inline]
    pub fn write(self, bytes: &mut impl Write, ip_version: IpVersion) -> Result<(), io::Error> {
        match self {
            Response::Connect(r) => {
                bytes.write_i32::<NetworkEndian>(0)?;
                bytes.write_i32::<NetworkEndian>(r.transaction_id.0)?;
                bytes.write_i64::<NetworkEndian>(r.connection_id.0)?;
            }
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
            }
            Response::Scrape(r) => {
                bytes.write_i32::<NetworkEndian>(2)?;
                bytes.write_i32::<NetworkEndian>(r.transaction_id.0)?;

                for torrent_stat in r.torrent_stats {
                    bytes.write_i32::<NetworkEndian>(torrent_stat.seeders.0)?;
                    bytes.write_i32::<NetworkEndian>(torrent_stat.completed.0)?;
                    bytes.write_i32::<NetworkEndian>(torrent_stat.leechers.0)?;
                }
            }
            Response::Error(r) => {
                bytes.write_i32::<NetworkEndian>(3)?;
                bytes.write_i32::<NetworkEndian>(r.transaction_id.0)?;

                bytes.write_all(r.message.as_bytes())?;
            }
        }

        Ok(())
    }

    #[inline]
    pub fn from_bytes(bytes: &[u8]) -> Result<Self, io::Error> {
        let mut cursor = Cursor::new(bytes);

        let action = cursor.read_i32::<NetworkEndian>()?;
        let transaction_id = cursor.read_i32::<NetworkEndian>()?;

        match action {
            // Connect
            0 => {
                let connection_id = cursor.read_i64::<NetworkEndian>()?;

                Ok((ConnectResponse {
                    connection_id: ConnectionId(connection_id),
                    transaction_id: TransactionId(transaction_id),
                })
                .into())
            }
            // Announce
            1 => {
                let announce_interval = cursor.read_i32::<NetworkEndian>()?;
                let leechers = cursor.read_i32::<NetworkEndian>()?;
                let seeders = cursor.read_i32::<NetworkEndian>()?;

                let position = cursor.position() as usize;
                let inner = cursor.into_inner();

                let peers = inner[position..]
                    .chunks_exact(6)
                    .map(|chunk| {
                        let ip_bytes: [u8; 4] = (&chunk[..4]).try_into().unwrap();
                        let ip_address = IpAddr::V4(Ipv4Addr::from(ip_bytes));
                        let port = (&chunk[4..]).read_u16::<NetworkEndian>().unwrap();

                        ResponsePeer {
                            ip_address,
                            port: Port(port),
                        }
                    })
                    .collect();

                Ok((AnnounceResponse {
                    transaction_id: TransactionId(transaction_id),
                    announce_interval: AnnounceInterval(announce_interval),
                    leechers: NumberOfPeers(leechers),
                    seeders: NumberOfPeers(seeders),
                    peers,
                })
                .into())
            }
            // Scrape
            2 => {
                let position = cursor.position() as usize;
                let inner = cursor.into_inner();

                let stats = inner[position..]
                    .chunks_exact(12)
                    .map(|chunk| {
                        let mut cursor: Cursor<&[u8]> = Cursor::new(&chunk[..]);

                        let seeders = cursor.read_i32::<NetworkEndian>().unwrap();
                        let downloads = cursor.read_i32::<NetworkEndian>().unwrap();
                        let leechers = cursor.read_i32::<NetworkEndian>().unwrap();

                        TorrentScrapeStatistics {
                            seeders: NumberOfPeers(seeders),
                            completed: NumberOfDownloads(downloads),
                            leechers: NumberOfPeers(leechers),
                        }
                    })
                    .collect();

                Ok((ScrapeResponse {
                    transaction_id: TransactionId(transaction_id),
                    torrent_stats: stats,
                })
                .into())
            }
            // Error
            3 => {
                let position = cursor.position() as usize;
                let inner = cursor.into_inner();

                Ok((ErrorResponse {
                    transaction_id: TransactionId(transaction_id),
                    message: String::from_utf8_lossy(&inner[position..]).into(),
                })
                .into())
            }
            // IPv6 announce
            4 => {
                let announce_interval = cursor.read_i32::<NetworkEndian>()?;
                let leechers = cursor.read_i32::<NetworkEndian>()?;
                let seeders = cursor.read_i32::<NetworkEndian>()?;

                let position = cursor.position() as usize;
                let inner = cursor.into_inner();

                let peers = inner[position..]
                    .chunks_exact(18)
                    .map(|chunk| {
                        let ip_bytes: [u8; 16] = (&chunk[..16]).try_into().unwrap();
                        let ip_address = IpAddr::V6(Ipv6Addr::from(ip_bytes));
                        let port = (&chunk[16..]).read_u16::<NetworkEndian>().unwrap();

                        ResponsePeer {
                            ip_address,
                            port: Port(port),
                        }
                    })
                    .collect();

                Ok((AnnounceResponse {
                    transaction_id: TransactionId(transaction_id),
                    announce_interval: AnnounceInterval(announce_interval),
                    leechers: NumberOfPeers(leechers),
                    seeders: NumberOfPeers(seeders),
                    peers,
                })
                .into())
            }
            _ => Ok((ErrorResponse {
                transaction_id: TransactionId(transaction_id),
                message: "Invalid action".to_string(),
            })
            .into()),
        }
    }
}

#[cfg(test)]
mod tests {
    use quickcheck_macros::quickcheck;

    use super::*;

    impl quickcheck::Arbitrary for TorrentScrapeStatistics {
        fn arbitrary(g: &mut quickcheck::Gen) -> Self {
            Self {
                seeders: NumberOfPeers(i32::arbitrary(g)),
                completed: NumberOfDownloads(i32::arbitrary(g)),
                leechers: NumberOfPeers(i32::arbitrary(g)),
            }
        }
    }

    impl quickcheck::Arbitrary for ConnectResponse {
        fn arbitrary(g: &mut quickcheck::Gen) -> Self {
            Self {
                connection_id: ConnectionId(i64::arbitrary(g)),
                transaction_id: TransactionId(i32::arbitrary(g)),
            }
        }
    }

    impl quickcheck::Arbitrary for AnnounceResponse {
        fn arbitrary(g: &mut quickcheck::Gen) -> Self {
            let peers = (0..u8::arbitrary(g))
                .map(|_| ResponsePeer::arbitrary(g))
                .collect();

            Self {
                transaction_id: TransactionId(i32::arbitrary(g)),
                announce_interval: AnnounceInterval(i32::arbitrary(g)),
                leechers: NumberOfPeers(i32::arbitrary(g)),
                seeders: NumberOfPeers(i32::arbitrary(g)),
                peers,
            }
        }
    }

    impl quickcheck::Arbitrary for ScrapeResponse {
        fn arbitrary(g: &mut quickcheck::Gen) -> Self {
            let torrent_stats = (0..u8::arbitrary(g))
                .map(|_| TorrentScrapeStatistics::arbitrary(g))
                .collect();

            Self {
                transaction_id: TransactionId(i32::arbitrary(g)),
                torrent_stats,
            }
        }
    }

    fn same_after_conversion(response: Response, ip_version: IpVersion) -> bool {
        let mut buf = Vec::new();

        response.clone().write(&mut buf, ip_version).unwrap();
        let r2 = Response::from_bytes(&buf[..]).unwrap();

        let success = response == r2;

        if !success {
            println!("before: {:#?}\nafter: {:#?}", response, r2);
        }

        success
    }

    #[quickcheck]
    fn test_connect_response_convert_identity(response: ConnectResponse) -> bool {
        same_after_conversion(response.into(), IpVersion::IPv4)
    }

    #[quickcheck]
    fn test_announce_response_convert_identity(data: (AnnounceResponse, IpVersion)) -> bool {
        let mut r = data.0;

        if data.1 == IpVersion::IPv4 {
            r.peers.retain(|peer| peer.ip_address.is_ipv4());
        } else {
            r.peers.retain(|peer| peer.ip_address.is_ipv6());
        }

        same_after_conversion(r.into(), data.1)
    }

    #[quickcheck]
    fn test_scrape_response_convert_identity(response: ScrapeResponse) -> bool {
        same_after_conversion(response.into(), IpVersion::IPv4)
    }
}
