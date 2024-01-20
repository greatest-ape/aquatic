use std::{borrow::Cow, fmt::Display, sync::OnceLock};

use compact_str::{format_compact, CompactString};
use regex::bytes::Regex;
use serde::{Deserialize, Serialize};
use zerocopy::{AsBytes, FromBytes, FromZeroes};

#[derive(
    Debug,
    Clone,
    Copy,
    PartialEq,
    Eq,
    PartialOrd,
    Ord,
    Hash,
    Serialize,
    Deserialize,
    AsBytes,
    FromBytes,
    FromZeroes,
)]
#[repr(transparent)]
pub struct PeerId(pub [u8; 20]);

impl PeerId {
    pub fn client(&self) -> PeerClient {
        PeerClient::from_peer_id(self)
    }
    pub fn first_8_bytes_hex(&self) -> CompactString {
        let mut buf = [0u8; 16];

        hex::encode_to_slice(&self.0[..8], &mut buf)
            .expect("PeerId.first_8_bytes_hex buffer too small");

        CompactString::from_utf8_lossy(&buf)
    }
}

#[non_exhaustive]
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum PeerClient {
    BitTorrent(CompactString),
    Deluge(CompactString),
    LibTorrentRakshasa(CompactString),
    LibTorrentRasterbar(CompactString),
    QBitTorrent(CompactString),
    Transmission(CompactString),
    UTorrent(CompactString),
    UTorrentEmbedded(CompactString),
    UTorrentMac(CompactString),
    UTorrentWeb(CompactString),
    Vuze(CompactString),
    WebTorrent(CompactString),
    WebTorrentDesktop(CompactString),
    Mainline(CompactString),
    OtherWithPrefixAndVersion {
        prefix: CompactString,
        version: CompactString,
    },
    OtherWithPrefix(CompactString),
    Other,
}

impl PeerClient {
    pub fn from_prefix_and_version(prefix: &[u8], version: &[u8]) -> Self {
        fn three_digits_plus_prerelease(v1: char, v2: char, v3: char, v4: char) -> CompactString {
            let prerelease: Cow<str> = match v4 {
                'd' | 'D' => " dev".into(),
                'a' | 'A' => " alpha".into(),
                'b' | 'B' => " beta".into(),
                'r' | 'R' => " rc".into(),
                's' | 'S' => " stable".into(),
                other => format_compact!("{}", other).into(),
            };

            format_compact!("{}.{}.{}{}", v1, v2, v3, prerelease)
        }

        fn webtorrent(v1: char, v2: char, v3: char, v4: char) -> CompactString {
            let major = if v1 == '0' {
                format_compact!("{}", v2)
            } else {
                format_compact!("{}{}", v1, v2)
            };

            let minor = if v3 == '0' {
                format_compact!("{}", v4)
            } else {
                format_compact!("{}{}", v3, v4)
            };

            format_compact!("{}.{}", major, minor)
        }

        if let [v1, v2, v3, v4] = version {
            let (v1, v2, v3, v4) = (*v1 as char, *v2 as char, *v3 as char, *v4 as char);

            match prefix {
                b"AZ" => Self::Vuze(format_compact!("{}.{}.{}.{}", v1, v2, v3, v4)),
                b"BT" => Self::BitTorrent(three_digits_plus_prerelease(v1, v2, v3, v4)),
                b"DE" => Self::Deluge(three_digits_plus_prerelease(v1, v2, v3, v4)),
                b"lt" => Self::LibTorrentRakshasa(format_compact!("{}.{}{}.{}", v1, v2, v3, v4)),
                b"LT" => Self::LibTorrentRasterbar(format_compact!("{}.{}{}.{}", v1, v2, v3, v4)),
                b"qB" => Self::QBitTorrent(format_compact!("{}.{}.{}", v1, v2, v3)),
                b"TR" => {
                    let v = match (v1, v2, v3, v4) {
                        ('0', '0', '0', v4) => format_compact!("0.{}", v4),
                        ('0', '0', v3, v4) => format_compact!("0.{}{}", v3, v4),
                        _ => format_compact!("{}.{}{}", v1, v2, v3),
                    };

                    Self::Transmission(v)
                }
                b"UE" => Self::UTorrentEmbedded(three_digits_plus_prerelease(v1, v2, v3, v4)),
                b"UM" => Self::UTorrentMac(three_digits_plus_prerelease(v1, v2, v3, v4)),
                b"UT" => Self::UTorrent(three_digits_plus_prerelease(v1, v2, v3, v4)),
                b"UW" => Self::UTorrentWeb(three_digits_plus_prerelease(v1, v2, v3, v4)),
                b"WD" => Self::WebTorrentDesktop(webtorrent(v1, v2, v3, v4)),
                b"WW" => Self::WebTorrent(webtorrent(v1, v2, v3, v4)),
                _ => Self::OtherWithPrefixAndVersion {
                    prefix: CompactString::from_utf8_lossy(prefix),
                    version: CompactString::from_utf8_lossy(version),
                },
            }
        } else {
            match (prefix, version) {
                (b"M", &[major, b'-', minor, b'-', patch, b'-']) => Self::Mainline(
                    format_compact!("{}.{}.{}", major as char, minor as char, patch as char),
                ),
                (b"M", &[major, b'-', minor1, minor2, b'-', patch]) => {
                    Self::Mainline(format_compact!(
                        "{}.{}{}.{}",
                        major as char,
                        minor1 as char,
                        minor2 as char,
                        patch as char
                    ))
                }
                _ => Self::OtherWithPrefixAndVersion {
                    prefix: CompactString::from_utf8_lossy(prefix),
                    version: CompactString::from_utf8_lossy(version),
                },
            }
        }
    }

    pub fn from_peer_id(peer_id: &PeerId) -> Self {
        static AZ_RE: OnceLock<Regex> = OnceLock::new();

        if let Some(caps) = AZ_RE
            .get_or_init(|| {
                Regex::new(r"^\-(?P<name>[a-zA-Z]{2})(?P<version>[0-9]{3}[0-9a-zA-Z])")
                    .expect("compile AZ_RE regex")
            })
            .captures(&peer_id.0)
        {
            return Self::from_prefix_and_version(&caps["name"], &caps["version"]);
        }

        static MAINLINE_RE: OnceLock<Regex> = OnceLock::new();

        if let Some(caps) = MAINLINE_RE
            .get_or_init(|| {
                Regex::new(r"^(?P<name>[a-zA-Z])(?P<version>[0-9\-]{6})\-")
                    .expect("compile MAINLINE_RE regex")
            })
            .captures(&peer_id.0)
        {
            return Self::from_prefix_and_version(&caps["name"], &caps["version"]);
        }

        static PREFIX_RE: OnceLock<Regex> = OnceLock::new();

        if let Some(caps) = PREFIX_RE
            .get_or_init(|| {
                Regex::new(r"^(?P<prefix>[a-zA-Z0-9\-]+)\-").expect("compile PREFIX_RE regex")
            })
            .captures(&peer_id.0)
        {
            return Self::OtherWithPrefix(CompactString::from_utf8_lossy(&caps["prefix"]));
        }

        Self::Other
    }
}

impl Display for PeerClient {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::BitTorrent(v) => write!(f, "BitTorrent {}", v.as_str()),
            Self::Deluge(v) => write!(f, "Deluge {}", v.as_str()),
            Self::LibTorrentRakshasa(v) => write!(f, "lt (rakshasa) {}", v.as_str()),
            Self::LibTorrentRasterbar(v) => write!(f, "lt (rasterbar) {}", v.as_str()),
            Self::QBitTorrent(v) => write!(f, "QBitTorrent {}", v.as_str()),
            Self::Transmission(v) => write!(f, "Transmission {}", v.as_str()),
            Self::UTorrent(v) => write!(f, "µTorrent {}", v.as_str()),
            Self::UTorrentEmbedded(v) => write!(f, "µTorrent Emb. {}", v.as_str()),
            Self::UTorrentMac(v) => write!(f, "µTorrent Mac {}", v.as_str()),
            Self::UTorrentWeb(v) => write!(f, "µTorrent Web {}", v.as_str()),
            Self::Vuze(v) => write!(f, "Vuze {}", v.as_str()),
            Self::WebTorrent(v) => write!(f, "WebTorrent {}", v.as_str()),
            Self::WebTorrentDesktop(v) => write!(f, "WebTorrent Desktop {}", v.as_str()),
            Self::Mainline(v) => write!(f, "Mainline {}", v.as_str()),
            Self::OtherWithPrefixAndVersion { prefix, version } => {
                write!(f, "Other ({}) ({})", prefix.as_str(), version.as_str())
            }
            Self::OtherWithPrefix(prefix) => write!(f, "Other ({})", prefix.as_str()),
            Self::Other => f.write_str("Other"),
        }
    }
}

#[cfg(feature = "quickcheck")]
impl quickcheck::Arbitrary for PeerId {
    fn arbitrary(g: &mut quickcheck::Gen) -> Self {
        let mut bytes = [0u8; 20];

        for byte in bytes.iter_mut() {
            *byte = u8::arbitrary(g);
        }

        Self(bytes)
    }
}

#[cfg(feature = "quickcheck")]
#[cfg(test)]
mod tests {
    use super::*;

    fn create_peer_id(bytes: &[u8]) -> PeerId {
        let mut peer_id = PeerId([0; 20]);

        let len = bytes.len();

        peer_id.0[..len].copy_from_slice(bytes);

        peer_id
    }

    #[test]
    fn test_client_from_peer_id() {
        assert_eq!(
            PeerClient::from_peer_id(&create_peer_id(b"-lt1234-k/asdh3")),
            PeerClient::LibTorrentRakshasa("1.23.4".into())
        );
        assert_eq!(
            PeerClient::from_peer_id(&create_peer_id(b"-DE123s-k/asdh3")),
            PeerClient::Deluge("1.2.3 stable".into())
        );
        assert_eq!(
            PeerClient::from_peer_id(&create_peer_id(b"-DE123r-k/asdh3")),
            PeerClient::Deluge("1.2.3 rc".into())
        );
        assert_eq!(
            PeerClient::from_peer_id(&create_peer_id(b"-UT123A-k/asdh3")),
            PeerClient::UTorrent("1.2.3 alpha".into())
        );
        assert_eq!(
            PeerClient::from_peer_id(&create_peer_id(b"-TR0012-k/asdh3")),
            PeerClient::Transmission("0.12".into())
        );
        assert_eq!(
            PeerClient::from_peer_id(&create_peer_id(b"-TR1212-k/asdh3")),
            PeerClient::Transmission("1.21".into())
        );
        assert_eq!(
            PeerClient::from_peer_id(&create_peer_id(b"-WW0102-k/asdh3")),
            PeerClient::WebTorrent("1.2".into())
        );
        assert_eq!(
            PeerClient::from_peer_id(&create_peer_id(b"-WW1302-k/asdh3")),
            PeerClient::WebTorrent("13.2".into())
        );
        assert_eq!(
            PeerClient::from_peer_id(&create_peer_id(b"-WW1324-k/asdh3")),
            PeerClient::WebTorrent("13.24".into())
        );
        assert_eq!(
            PeerClient::from_peer_id(&create_peer_id(b"M1-2-3--k/asdh3")),
            PeerClient::Mainline("1.2.3".into())
        );
        assert_eq!(
            PeerClient::from_peer_id(&create_peer_id(b"M1-23-4-k/asdh3")),
            PeerClient::Mainline("1.23.4".into())
        );
        assert_eq!(
            PeerClient::from_peer_id(&create_peer_id(b"S3-k/asdh3")),
            PeerClient::OtherWithPrefix("S3".into())
        );
    }
}
