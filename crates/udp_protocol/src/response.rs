use std::borrow::Cow;
use std::io::{self, Write};
use std::mem::size_of;

use byteorder::{NetworkEndian, WriteBytesExt};
use zerocopy::{AsBytes, FromBytes, FromZeroes};

use super::common::*;

#[derive(PartialEq, Eq, Clone, Debug)]
pub enum Response {
    Connect(ConnectResponse),
    AnnounceIpv4(AnnounceResponse<Ipv4AddrBytes>),
    AnnounceIpv6(AnnounceResponse<Ipv6AddrBytes>),
    Scrape(ScrapeResponse),
    Error(ErrorResponse),
}

impl Response {
    #[inline]
    pub fn write_bytes(&self, bytes: &mut impl Write) -> Result<(), io::Error> {
        match self {
            Response::Connect(r) => r.write_bytes(bytes),
            Response::AnnounceIpv4(r) => r.write_bytes(bytes),
            Response::AnnounceIpv6(r) => r.write_bytes(bytes),
            Response::Scrape(r) => r.write_bytes(bytes),
            Response::Error(r) => r.write_bytes(bytes),
        }
    }

    #[inline]
    pub fn parse_bytes(mut bytes: &[u8], ipv4: bool) -> Result<Self, io::Error> {
        let action = read_i32_ne(&mut bytes)?;

        match action.get() {
            // Connect
            0 => Ok(Response::Connect(
                ConnectResponse::read_from_prefix(bytes).ok_or_else(invalid_data)?,
            )),
            // Announce
            1 if ipv4 => {
                let fixed =
                    AnnounceResponseFixedData::read_from_prefix(bytes).ok_or_else(invalid_data)?;

                let peers = if let Some(bytes) = bytes.get(size_of::<AnnounceResponseFixedData>()..)
                {
                    Vec::from(
                        ResponsePeer::<Ipv4AddrBytes>::slice_from(bytes)
                            .ok_or_else(invalid_data)?,
                    )
                } else {
                    Vec::new()
                };

                Ok(Response::AnnounceIpv4(AnnounceResponse { fixed, peers }))
            }
            1 if !ipv4 => {
                let fixed =
                    AnnounceResponseFixedData::read_from_prefix(bytes).ok_or_else(invalid_data)?;

                let peers = if let Some(bytes) = bytes.get(size_of::<AnnounceResponseFixedData>()..)
                {
                    Vec::from(
                        ResponsePeer::<Ipv6AddrBytes>::slice_from(bytes)
                            .ok_or_else(invalid_data)?,
                    )
                } else {
                    Vec::new()
                };

                Ok(Response::AnnounceIpv6(AnnounceResponse { fixed, peers }))
            }
            // Scrape
            2 => {
                let transaction_id = read_i32_ne(&mut bytes).map(TransactionId)?;
                let torrent_stats =
                    Vec::from(TorrentScrapeStatistics::slice_from(bytes).ok_or_else(invalid_data)?);

                Ok((ScrapeResponse {
                    transaction_id,
                    torrent_stats,
                })
                .into())
            }
            // Error
            3 => {
                let transaction_id = read_i32_ne(&mut bytes).map(TransactionId)?;
                let message = String::from_utf8_lossy(bytes).into_owned().into();

                Ok((ErrorResponse {
                    transaction_id,
                    message,
                })
                .into())
            }
            _ => Err(invalid_data()),
        }
    }
}

impl From<ConnectResponse> for Response {
    fn from(r: ConnectResponse) -> Self {
        Self::Connect(r)
    }
}

impl From<AnnounceResponse<Ipv4AddrBytes>> for Response {
    fn from(r: AnnounceResponse<Ipv4AddrBytes>) -> Self {
        Self::AnnounceIpv4(r)
    }
}

impl From<AnnounceResponse<Ipv6AddrBytes>> for Response {
    fn from(r: AnnounceResponse<Ipv6AddrBytes>) -> Self {
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

#[derive(PartialEq, Eq, Clone, Copy, Debug, AsBytes, FromBytes, FromZeroes)]
#[repr(C, packed)]
pub struct ConnectResponse {
    pub transaction_id: TransactionId,
    pub connection_id: ConnectionId,
}

impl ConnectResponse {
    #[inline]
    pub fn write_bytes(&self, bytes: &mut impl Write) -> Result<(), io::Error> {
        bytes.write_i32::<NetworkEndian>(0)?;
        bytes.write_all(self.as_bytes())?;

        Ok(())
    }
}

#[derive(PartialEq, Eq, Clone, Debug)]
pub struct AnnounceResponse<I: Ip> {
    pub fixed: AnnounceResponseFixedData,
    pub peers: Vec<ResponsePeer<I>>,
}

impl<I: Ip> AnnounceResponse<I> {
    pub fn empty() -> Self {
        Self {
            fixed: FromZeroes::new_zeroed(),
            peers: Default::default(),
        }
    }

    #[inline]
    pub fn write_bytes(&self, bytes: &mut impl Write) -> Result<(), io::Error> {
        bytes.write_i32::<NetworkEndian>(1)?;
        bytes.write_all(self.fixed.as_bytes())?;
        bytes.write_all((*self.peers.as_slice()).as_bytes())?;

        Ok(())
    }
}

#[derive(PartialEq, Eq, Clone, Copy, Debug, AsBytes, FromBytes, FromZeroes)]
#[repr(C, packed)]
pub struct AnnounceResponseFixedData {
    pub transaction_id: TransactionId,
    pub announce_interval: AnnounceInterval,
    pub leechers: NumberOfPeers,
    pub seeders: NumberOfPeers,
}

#[derive(PartialEq, Eq, Clone, Debug)]
pub struct ScrapeResponse {
    pub transaction_id: TransactionId,
    pub torrent_stats: Vec<TorrentScrapeStatistics>,
}

impl ScrapeResponse {
    #[inline]
    pub fn write_bytes(&self, bytes: &mut impl Write) -> Result<(), io::Error> {
        bytes.write_i32::<NetworkEndian>(2)?;
        bytes.write_all(self.transaction_id.as_bytes())?;
        bytes.write_all((*self.torrent_stats.as_slice()).as_bytes())?;

        Ok(())
    }
}

#[derive(PartialEq, Eq, Debug, Copy, Clone, AsBytes, FromBytes, FromZeroes)]
#[repr(C, packed)]
pub struct TorrentScrapeStatistics {
    pub seeders: NumberOfPeers,
    pub completed: NumberOfDownloads,
    pub leechers: NumberOfPeers,
}

#[derive(PartialEq, Eq, Clone, Debug)]
pub struct ErrorResponse {
    pub transaction_id: TransactionId,
    pub message: Cow<'static, str>,
}

impl ErrorResponse {
    #[inline]
    pub fn write_bytes(&self, bytes: &mut impl Write) -> Result<(), io::Error> {
        bytes.write_i32::<NetworkEndian>(3)?;
        bytes.write_all(self.transaction_id.as_bytes())?;
        bytes.write_all(self.message.as_bytes())?;

        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use quickcheck_macros::quickcheck;
    use zerocopy::network_endian::I32;
    use zerocopy::network_endian::I64;

    use super::*;

    impl quickcheck::Arbitrary for Ipv4AddrBytes {
        fn arbitrary(g: &mut quickcheck::Gen) -> Self {
            Self([
                u8::arbitrary(g),
                u8::arbitrary(g),
                u8::arbitrary(g),
                u8::arbitrary(g),
            ])
        }
    }

    impl quickcheck::Arbitrary for Ipv6AddrBytes {
        fn arbitrary(g: &mut quickcheck::Gen) -> Self {
            let mut bytes = [0; 16];

            for byte in bytes.iter_mut() {
                *byte = u8::arbitrary(g)
            }

            Self(bytes)
        }
    }

    impl quickcheck::Arbitrary for TorrentScrapeStatistics {
        fn arbitrary(g: &mut quickcheck::Gen) -> Self {
            Self {
                seeders: NumberOfPeers(I32::new(i32::arbitrary(g))),
                completed: NumberOfDownloads(I32::new(i32::arbitrary(g))),
                leechers: NumberOfPeers(I32::new(i32::arbitrary(g))),
            }
        }
    }

    impl quickcheck::Arbitrary for ConnectResponse {
        fn arbitrary(g: &mut quickcheck::Gen) -> Self {
            Self {
                connection_id: ConnectionId(I64::new(i64::arbitrary(g))),
                transaction_id: TransactionId(I32::new(i32::arbitrary(g))),
            }
        }
    }

    impl<I: Ip + quickcheck::Arbitrary> quickcheck::Arbitrary for AnnounceResponse<I> {
        fn arbitrary(g: &mut quickcheck::Gen) -> Self {
            let peers = (0..u8::arbitrary(g))
                .map(|_| ResponsePeer::arbitrary(g))
                .collect();

            Self {
                fixed: AnnounceResponseFixedData {
                    transaction_id: TransactionId(I32::new(i32::arbitrary(g))),
                    announce_interval: AnnounceInterval(I32::new(i32::arbitrary(g))),
                    leechers: NumberOfPeers(I32::new(i32::arbitrary(g))),
                    seeders: NumberOfPeers(I32::new(i32::arbitrary(g))),
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
                transaction_id: TransactionId(I32::new(i32::arbitrary(g))),
                torrent_stats,
            }
        }
    }

    fn same_after_conversion(response: Response, ipv4: bool) -> bool {
        let mut buf = Vec::new();

        response.clone().write_bytes(&mut buf).unwrap();
        let r2 = Response::parse_bytes(&buf[..], ipv4).unwrap();

        let success = response == r2;

        if !success {
            ::pretty_assertions::assert_eq!(response, r2);
        }

        success
    }

    #[quickcheck]
    fn test_connect_response_convert_identity(response: ConnectResponse) -> bool {
        same_after_conversion(response.into(), true)
    }

    #[quickcheck]
    fn test_announce_response_ipv4_convert_identity(
        response: AnnounceResponse<Ipv4AddrBytes>,
    ) -> bool {
        same_after_conversion(response.into(), true)
    }

    #[quickcheck]
    fn test_announce_response_ipv6_convert_identity(
        response: AnnounceResponse<Ipv6AddrBytes>,
    ) -> bool {
        same_after_conversion(response.into(), false)
    }

    #[quickcheck]
    fn test_scrape_response_convert_identity(response: ScrapeResponse) -> bool {
        same_after_conversion(response.into(), true)
    }
}
