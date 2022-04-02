use std::str::FromStr;

use serde::{Deserialize, Serialize};

use super::utils::*;

#[derive(Debug, Clone, Copy, Hash, PartialEq, Eq, Serialize, Deserialize)]
#[serde(transparent)]
pub struct PeerId(
    #[serde(
        serialize_with = "serialize_20_bytes",
        deserialize_with = "deserialize_20_bytes"
    )]
    pub [u8; 20],
);

#[derive(Debug, Clone, Copy, Hash, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(transparent)]
pub struct InfoHash(
    #[serde(
        serialize_with = "serialize_20_bytes",
        deserialize_with = "deserialize_20_bytes"
    )]
    pub [u8; 20],
);

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AnnounceEvent {
    Started,
    Stopped,
    Completed,
    Empty,
}

impl Default for AnnounceEvent {
    fn default() -> Self {
        Self::Empty
    }
}

impl FromStr for AnnounceEvent {
    type Err = String;

    fn from_str(value: &str) -> std::result::Result<Self, String> {
        match value {
            "started" => Ok(Self::Started),
            "stopped" => Ok(Self::Stopped),
            "completed" => Ok(Self::Completed),
            "empty" => Ok(Self::Empty),
            value => Err(format!("Unknown value: {}", value)),
        }
    }
}

impl AnnounceEvent {
    pub fn as_str(&self) -> Option<&str> {
        match self {
            Self::Started => Some("started"),
            Self::Stopped => Some("stopped"),
            Self::Completed => Some("completed"),
            Self::Empty => None,
        }
    }
}

#[cfg(test)]
impl quickcheck::Arbitrary for InfoHash {
    fn arbitrary(g: &mut quickcheck::Gen) -> Self {
        let mut arr = [b'0'; 20];

        for byte in arr.iter_mut() {
            *byte = u8::arbitrary(g);
        }

        Self(arr)
    }
}

#[cfg(test)]
impl quickcheck::Arbitrary for PeerId {
    fn arbitrary(g: &mut quickcheck::Gen) -> Self {
        let mut arr = [b'0'; 20];

        for byte in arr.iter_mut() {
            *byte = u8::arbitrary(g);
        }

        Self(arr)
    }
}

#[cfg(test)]
impl quickcheck::Arbitrary for AnnounceEvent {
    fn arbitrary(g: &mut quickcheck::Gen) -> Self {
        match (bool::arbitrary(g), bool::arbitrary(g)) {
            (false, false) => Self::Started,
            (true, false) => Self::Started,
            (false, true) => Self::Completed,
            (true, true) => Self::Empty,
        }
    }
}
