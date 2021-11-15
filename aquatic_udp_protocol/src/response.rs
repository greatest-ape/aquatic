use std::borrow::Cow;
use std::io::{self, Write};
use std::mem::size_of;

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
    pub action: ConnectAction,
    pub connection_id: ConnectionId,
    pub transaction_id: TransactionId,
}

#[derive(PartialEq, Eq, Clone, Debug, AsBytes, FromBytes, Unaligned)]
#[repr(C)]
pub struct AnnounceResponseIpv4Fixed {
    pub action: AnnounceAction,
    pub transaction_id: TransactionId,
    pub announce_interval: AnnounceInterval,
    pub leechers: NumberOfPeers,
    pub seeders: NumberOfPeers,
}

#[derive(PartialEq, Eq, Clone, Debug, AsBytes, FromBytes, Unaligned)]
#[repr(C)]
pub struct AnnounceResponseIpv6Fixed {
    pub action: AnnounceIpv6Action,
    pub transaction_id: TransactionId,
    pub announce_interval: AnnounceInterval,
    pub leechers: NumberOfPeers,
    pub seeders: NumberOfPeers,
}

#[derive(PartialEq, Eq, Clone, Debug)]
pub struct AnnounceResponseIpv4 {
    pub fixed: AnnounceResponseIpv4Fixed,
    pub peers: Vec<ResponsePeerIpv4>,
}

#[derive(PartialEq, Eq, Clone, Debug)]
pub struct AnnounceResponseIpv6 {
    pub fixed: AnnounceResponseIpv6Fixed,
    pub peers: Vec<ResponsePeerIpv6>,
}

#[derive(PartialEq, Eq, Clone, Debug, AsBytes, FromBytes, Unaligned)]
#[repr(C)]
pub struct ScrapeResponseFixed {
    pub action: ScrapeAction,
    pub transaction_id: TransactionId,
}

#[derive(PartialEq, Eq, Clone, Debug)]
pub struct ScrapeResponse {
    pub fixed: ScrapeResponseFixed,
    pub torrent_stats: Vec<TorrentScrapeStatistics>,
}

#[derive(PartialEq, Eq, Clone, Debug, AsBytes, FromBytes, Unaligned)]
#[repr(C)]
pub struct ErrorResponseFixed {
    pub action: ErrorAction,
    pub transaction_id: TransactionId,
}

#[derive(PartialEq, Eq, Clone, Debug)]
pub struct ErrorResponse {
    pub fixed: ErrorResponseFixed,
    pub message: Cow<'static, str>,
}

#[derive(PartialEq, Eq, Clone, Debug)]
pub enum Response {
    Connect(ConnectResponse),
    AnnounceIpv4(AnnounceResponseIpv4),
    AnnounceIpv6(AnnounceResponseIpv6),
    Scrape(ScrapeResponse),
    Error(ErrorResponse),
}

impl From<ConnectResponse> for Response {
    fn from(r: ConnectResponse) -> Self {
        Self::Connect(r)
    }
}

impl From<AnnounceResponseIpv4> for Response {
    fn from(r: AnnounceResponseIpv4) -> Self {
        Self::AnnounceIpv4(r)
    }
}

impl From<AnnounceResponseIpv6> for Response {
    fn from(r: AnnounceResponseIpv6) -> Self {
        Self::AnnounceIpv6(r)
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
    pub fn write(self, bytes: &mut impl Write) -> Result<(), io::Error> {
        match self {
            Response::Connect(r) => {
                bytes.write(r.as_bytes())?;
            }
            Response::AnnounceIpv4(r) => {
                bytes.write(r.fixed.as_bytes())?;

                for peer in r.peers {
                    bytes.write(peer.as_bytes())?;
                }
            }
            Response::AnnounceIpv6(r) => {
                bytes.write(r.fixed.as_bytes())?;

                for peer in r.peers {
                    bytes.write(peer.as_bytes())?;
                }
            }
            Response::Scrape(r) => {
                bytes.write(r.fixed.as_bytes())?;

                for torrent_stat in r.torrent_stats {
                    bytes.write(torrent_stat.as_bytes())?;
                }
            }
            Response::Error(r) => {
                bytes.write(r.fixed.as_bytes())?;
                bytes.write(r.message.as_bytes())?;
            }
        }

        Ok(())
    }

    #[inline]
    pub fn from_bytes(mut bytes: &[u8]) -> Option<Self> {
        let action: I32<NetworkEndian> = FromBytes::read_from_prefix(bytes)?;

        match action.get() {
            // Connect
            0 => ConnectResponse::read_from(bytes).map(|r| r.into()),
            // Announce
            1 => {
                let fixed = AnnounceResponseIpv4Fixed::read_from_prefix(bytes)?;
                bytes = &bytes[size_of::<AnnounceResponseIpv4Fixed>()..];

                let peers = bytes[..]
                    .chunks_exact(6)
                    .map(|chunk| ResponsePeerIpv4::read_from(chunk).unwrap())
                    .collect();

                Some((AnnounceResponseIpv4 { fixed, peers }).into())
            }
            // Scrape
            2 => {
                let fixed = ScrapeResponseFixed::read_from_prefix(bytes)?;
                bytes = &bytes[size_of::<ScrapeResponseFixed>()..];

                let torrent_stats = bytes
                    .chunks_exact(12)
                    .map(|chunk| TorrentScrapeStatistics::read_from(chunk).unwrap())
                    .collect();

                Some(
                    (ScrapeResponse {
                        fixed,
                        torrent_stats,
                    })
                    .into(),
                )
            }
            // Error
            3 => {
                let fixed = ErrorResponseFixed::read_from_prefix(bytes)?;
                bytes = &bytes[size_of::<ErrorResponseFixed>()..];

                Some(
                    (ErrorResponse {
                        fixed,
                        message: String::from_utf8_lossy(&bytes).into_owned().into(),
                    })
                    .into(),
                )
            }
            // IPv6 announce
            4 => {
                let fixed = AnnounceResponseIpv6Fixed::read_from_prefix(bytes)?;
                bytes = &bytes[size_of::<AnnounceResponseIpv6Fixed>()..];

                let peers = bytes
                    .chunks_exact(18)
                    .map(|chunk| ResponsePeerIpv6::read_from(chunk).unwrap())
                    .collect();

                Some((AnnounceResponseIpv6 { fixed, peers }).into())
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
                action: ConnectAction::new(),
                connection_id: ConnectionId(i64::arbitrary(g).into()),
                transaction_id: TransactionId(i32::arbitrary(g).into()),
            }
        }
    }

    impl quickcheck::Arbitrary for AnnounceResponseIpv4 {
        fn arbitrary(g: &mut quickcheck::Gen) -> Self {
            let peers = (0..u8::arbitrary(g))
                .map(|_| ResponsePeerIpv4::arbitrary(g))
                .collect();

            Self {
                fixed: AnnounceResponseIpv4Fixed {
                    action: AnnounceAction::new(),
                    transaction_id: TransactionId(i32::arbitrary(g).into()),
                    announce_interval: AnnounceInterval(i32::arbitrary(g).into()),
                    leechers: NumberOfPeers(i32::arbitrary(g).into()),
                    seeders: NumberOfPeers(i32::arbitrary(g).into()),
                },
                peers,
            }
        }
    }

    impl quickcheck::Arbitrary for AnnounceResponseIpv6 {
        fn arbitrary(g: &mut quickcheck::Gen) -> Self {
            let peers = (0..u8::arbitrary(g))
                .map(|_| ResponsePeerIpv6::arbitrary(g))
                .collect();

            Self {
                fixed: AnnounceResponseIpv6Fixed {
                    action: AnnounceIpv6Action::new(),
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
                fixed: ScrapeResponseFixed {
                    action: ScrapeAction::new(),
                    transaction_id: TransactionId(i32::arbitrary(g).into()),
                },
                torrent_stats,
            }
        }
    }

    fn same_after_conversion(response: Response) -> bool {
        let mut buf = Vec::new();

        response.clone().write(&mut buf).unwrap();
        let r2 = Response::from_bytes(&buf[..]).unwrap();

        let success = response == r2;

        if !success {
            println!("before: {:#?}\nafter: {:#?}", response, r2);
        }

        success
    }

    #[quickcheck]
    fn test_connect_response_convert_identity(response: ConnectResponse) -> bool {
        same_after_conversion(response.into())
    }

    #[quickcheck]
    fn test_announce_response_ipv4_convert_identity(response: AnnounceResponseIpv4) -> bool {
        same_after_conversion(response.into())
    }

    #[quickcheck]
    fn test_announce_response_ipv6_convert_identity(response: AnnounceResponseIpv6) -> bool {
        same_after_conversion(response.into())
    }

    #[quickcheck]
    fn test_scrape_response_convert_identity(response: ScrapeResponse) -> bool {
        same_after_conversion(response.into())
    }
}
