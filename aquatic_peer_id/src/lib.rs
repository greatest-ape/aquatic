use std::{fmt::Display, sync::OnceLock};

use compact_str::CompactString;
use regex::bytes::Regex;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
pub struct PeerId(pub [u8; 20]);

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
    pub fn from_prefix_and_version(prefix: &[u8], version: &[u8]) -> Option<Self> {
        let version = CompactString::from_utf8(version).ok()?;

        match prefix {
            b"AZ" => Some(Self::Vuze(version)),
            b"BT" => Some(Self::BitTorrent(version)),
            b"DE" => Some(Self::Deluge(version)),
            b"lt" => Some(Self::LibTorrentRakshasa(version)),
            b"LT" => Some(Self::LibTorrentRasterbar(version)),
            b"qB" => Some(Self::QBitTorrent(version)),
            b"TR" => Some(Self::Transmission(version)),
            b"UE" => Some(Self::UTorrentEmbedded(version)),
            b"UM" => Some(Self::UTorrentMac(version)),
            b"UT" => Some(Self::UTorrent(version)),
            b"UW" => Some(Self::UTorrentWeb(version)),
            b"WD" => Some(Self::WebTorrentDesktop(version)),
            b"WW" => Some(Self::WebTorrent(version)),
            b"M" => Some(Self::Mainline(version)),
            name => Some(Self::OtherWithPrefixAndVersion {
                prefix: CompactString::from_utf8(name).ok()?,
                version,
            }),
        }
    }

    pub fn from_peer_id(peer_id: PeerId) -> Self {
        static AZ_RE: OnceLock<Regex> = OnceLock::new();

        if let Some(caps) = AZ_RE
            .get_or_init(|| {
                Regex::new(r"^\-(?P<name>[a-zA-Z]{2})(?P<version>[0-9A-Z]{4})")
                    .expect("compile AZ_RE regex")
            })
            .captures(&peer_id.0)
        {
            if let Some(client) = Self::from_prefix_and_version(&caps["name"], &caps["version"]) {
                return client;
            }
        }

        static MAINLINE_RE: OnceLock<Regex> = OnceLock::new();

        if let Some(caps) = MAINLINE_RE
            .get_or_init(|| {
                Regex::new(r"^(?P<name>[a-zA-Z])(?P<version>[0-9\-]{6})\-")
                    .expect("compile MAINLINE_RE regex")
            })
            .captures(&peer_id.0)
        {
            if let Some(client) = Self::from_prefix_and_version(&caps["name"], &caps["version"]) {
                return client;
            }
        }

        static PREFIX_RE: OnceLock<Regex> = OnceLock::new();

        if let Some(caps) = PREFIX_RE
            .get_or_init(|| {
                Regex::new(r"^(?P<prefix>[a-zA-Z0-9\-]*)\-").expect("compile PREFIX_RE regex")
            })
            .captures(&peer_id.0)
        {
            if let Ok(prefix) = CompactString::from_utf8(&caps["prefix"]) {
                return Self::OtherWithPrefix(prefix);
            }
        }

        Self::Other
    }
}

impl Display for PeerClient {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::BitTorrent(v) => write!(f, "BitTorrent ({})", v.as_str()),
            Self::Deluge(v) => write!(f, "Deluge ({})", v.as_str()),
            Self::LibTorrentRakshasa(v) => write!(f, "libTorrent (rakshasa) ({})", v.as_str()),
            Self::LibTorrentRasterbar(v) => write!(f, "libtorrent (rasterbar) ({})", v.as_str()),
            Self::QBitTorrent(v) => write!(f, "QBitTorrent ({})", v.as_str()),
            Self::Transmission(v) => write!(f, "Transmission ({})", v.as_str()),
            Self::UTorrent(v) => write!(f, "uTorrent ({})", v.as_str()),
            Self::UTorrentEmbedded(v) => write!(f, "uTorrent Embedded ({})", v.as_str()),
            Self::UTorrentMac(v) => write!(f, "uTorrent Mac ({})", v.as_str()),
            Self::UTorrentWeb(v) => write!(f, "uTorrent Web ({})", v.as_str()),
            Self::Vuze(v) => write!(f, "Vuze ({})", v.as_str()),
            Self::WebTorrent(v) => write!(f, "WebTorrent ({})", v.as_str()),
            Self::WebTorrentDesktop(v) => write!(f, "WebTorrent Desktop ({})", v.as_str()),
            Self::Mainline(v) => write!(f, "Mainline ({})", v.as_str()),
            Self::OtherWithPrefixAndVersion { prefix, version } => {
                write!(f, "Other ({}) ({})", prefix.as_str(), version.as_str())
            }
            Self::OtherWithPrefix(prefix) => write!(f, "Other ({})", prefix.as_str()),
            Self::Other => f.write_str("Other"),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn create_peer_id(bytes: &[u8]) -> PeerId {
        let mut peer_id = PeerId([0; 20]);

        let len = bytes.len();

        (&mut peer_id.0[..len]).copy_from_slice(bytes);

        peer_id
    }

    #[test]
    fn test_client_from_peer_id() {
        assert_eq!(
            PeerClient::from_peer_id(create_peer_id(b"-lt1234-k/asdh3")),
            PeerClient::LibTorrentRakshasa("1234".into())
        );
        assert_eq!(
            PeerClient::from_peer_id(create_peer_id(b"M1-2-3--k/asdh3")),
            PeerClient::Mainline("1-2-3-".into())
        );
        assert_eq!(
            PeerClient::from_peer_id(create_peer_id(b"M1-23-4-k/asdh3")),
            PeerClient::Mainline("1-23-4".into())
        );
        assert_eq!(
            PeerClient::from_peer_id(create_peer_id(b"S3-k/asdh3")),
            PeerClient::OtherWithPrefix("S3".into())
        );
    }
}
