use std::borrow::Cow;
use std::convert::TryInto;
use std::io::{self, Write};
use std::mem::size_of;
use std::net::{IpAddr, Ipv4Addr, Ipv6Addr};

use zerocopy::{AsBytes, FromBytes, NetworkEndian, Unaligned, I32};

use super::common::*;

#[derive(PartialEq, Eq, Debug, Copy, Clone, AsBytes, FromBytes, Unaligned)]
#[repr(C)]
pub struct TorrentScrapeStatistics {
    pub seeders: NumberOfPeers,
    pub completed: NumberOfDownloads,
    pub leechers: NumberOfPeers,
}

#[derive(PartialEq, Eq, Clone, Debug, AsBytes, FromBytes, Unaligned)]
#[repr(C)]
pub struct ConnectResponse {
    pub connection_id: ConnectionId,
    pub transaction_id: TransactionId,
}

#[derive(PartialEq, Eq, Clone, Debug, AsBytes, FromBytes, Unaligned)]
#[repr(C)]
pub struct AnnounceResponseFixed {
    pub transaction_id: TransactionId,
    pub announce_interval: AnnounceInterval,
    pub leechers: NumberOfPeers,
    pub seeders: NumberOfPeers,
}

#[derive(PartialEq, Eq, Clone, Debug)]
pub struct AnnounceResponse {
    pub fixed: AnnounceResponseFixed,
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
    pub message: Cow<'static, str>,
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
                let action = ConnectAction::new();
                bytes.write(action.as_bytes())?;

                bytes.write(r.as_bytes())?;
            }
            Response::Announce(r) => {
                if ip_version == IpVersion::IPv4 {
                    let action = AnnounceAction::new();
                    bytes.write(action.as_bytes())?;

                    bytes.write(r.fixed.as_bytes())?;

                    // Silently ignore peers with wrong IP version
                    for peer in r.peers {
                        if let IpAddr::V4(ip) = peer.ip_address {
                            bytes.write_all(&ip.octets())?;
                            bytes.write(peer.port.as_bytes())?;
                        }
                    }
                } else {
                    let action = AnnounceIpv6Action::new();
                    bytes.write(action.as_bytes())?;

                    bytes.write(r.fixed.as_bytes())?;

                    // Silently ignore peers with wrong IP version
                    for peer in r.peers {
                        if let IpAddr::V6(ip) = peer.ip_address {
                            bytes.write_all(&ip.octets())?;
                            bytes.write(peer.port.as_bytes())?;
                        }
                    }
                }
            }
            Response::Scrape(r) => {
                let action = ScrapeAction::new();
                bytes.write(action.as_bytes())?;

                bytes.write(r.transaction_id.as_bytes())?;

                for torrent_stat in r.torrent_stats {
                    bytes.write(torrent_stat.as_bytes())?;
                }
            }
            Response::Error(r) => {
                let action = ErrorAction::new();
                bytes.write(action.as_bytes())?;

                bytes.write(r.transaction_id.as_bytes())?;

                bytes.write_all(r.message.as_bytes())?;
            }
        }

        Ok(())
    }

    #[inline]
    pub fn from_bytes(mut bytes: &[u8]) -> Option<Self> {
        let action: I32<NetworkEndian> = FromBytes::read_from_prefix(bytes)?;
        bytes = &bytes[size_of::<i32>()..];

        match action.get() {
            // Connect
            0 => ConnectResponse::read_from(bytes).map(|r| r.into()),
            // Announce
            1 => {
                let fixed = AnnounceResponseFixed::read_from_prefix(bytes)?;
                bytes = &bytes[size_of::<AnnounceResponseFixed>()..];

                let peers = bytes[..]
                    .chunks_exact(6)
                    .map(|chunk| {
                        let ip_bytes: [u8; 4] = (&chunk[..4]).try_into().unwrap();
                        let ip_address = IpAddr::V4(Ipv4Addr::from(ip_bytes));
                        let port = Port::read_from(&chunk[4..]).unwrap();

                        ResponsePeer { ip_address, port }
                    })
                    .collect();

                Some((AnnounceResponse { fixed, peers }).into())
            }
            // Scrape
            2 => {
                let transaction_id = TransactionId::read_from_prefix(bytes)?;
                bytes = &bytes[size_of::<TransactionId>()..];

                let torrent_stats = bytes
                    .chunks_exact(12)
                    .map(|chunk| TorrentScrapeStatistics::read_from(chunk).unwrap())
                    .collect();

                Some(
                    (ScrapeResponse {
                        transaction_id,
                        torrent_stats,
                    })
                    .into(),
                )
            }
            // Error
            3 => {
                let transaction_id = TransactionId::read_from_prefix(bytes)?;
                bytes = &bytes[size_of::<TransactionId>()..];

                Some(
                    (ErrorResponse {
                        transaction_id,
                        message: String::from_utf8_lossy(&bytes).into_owned().into(),
                    })
                    .into(),
                )
            }
            // IPv6 announce
            4 => {
                let fixed = AnnounceResponseFixed::read_from_prefix(bytes)?;
                bytes = &bytes[size_of::<AnnounceResponseFixed>()..];

                let peers = bytes
                    .chunks_exact(18)
                    .map(|chunk| {
                        let ip_bytes: [u8; 16] = (&chunk[..16]).try_into().unwrap();
                        let ip_address = IpAddr::V6(Ipv6Addr::from(ip_bytes));
                        let port = Port::read_from(&chunk[16..]).unwrap();

                        ResponsePeer { ip_address, port }
                    })
                    .collect();

                Some((AnnounceResponse { fixed, peers }).into())
            }
            _ => None,
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
                seeders: NumberOfPeers(i32::arbitrary(g).into()),
                completed: NumberOfDownloads(i32::arbitrary(g).into()),
                leechers: NumberOfPeers(i32::arbitrary(g).into()),
            }
        }
    }

    impl quickcheck::Arbitrary for ConnectResponse {
        fn arbitrary(g: &mut quickcheck::Gen) -> Self {
            Self {
                connection_id: ConnectionId(i64::arbitrary(g).into()),
                transaction_id: TransactionId(i32::arbitrary(g).into()),
            }
        }
    }

    impl quickcheck::Arbitrary for AnnounceResponse {
        fn arbitrary(g: &mut quickcheck::Gen) -> Self {
            let peers = (0..u8::arbitrary(g))
                .map(|_| ResponsePeer::arbitrary(g))
                .collect();

            Self {
                fixed: AnnounceResponseFixed {
                    transaction_id: TransactionId(i32::arbitrary(g).into()),
                    announce_interval: AnnounceInterval(i32::arbitrary(g).into()),
                    leechers: NumberOfPeers(i32::arbitrary(g).into()),
                    seeders: NumberOfPeers(i32::arbitrary(g).into()),
                },
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
                transaction_id: TransactionId(i32::arbitrary(g).into()),
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
