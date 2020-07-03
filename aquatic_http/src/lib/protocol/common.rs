use std::str::FromStr;

use serde::Serialize;

use super::utils::*;


#[derive(Debug, Clone, Hash, PartialEq, Eq, Serialize)]
#[serde(transparent)]
pub struct PeerId(
    #[serde(
        serialize_with = "serialize_20_bytes",
    )]
    pub [u8; 20]
);


#[derive(Debug, Clone, Hash, PartialEq, Eq, Serialize)]
#[serde(transparent)]
pub struct InfoHash(
    #[serde(
        serialize_with = "serialize_20_bytes",
    )]
    pub [u8; 20]
);


#[derive(Debug, Clone)]
pub enum AnnounceEvent {
    Started,
    Stopped,
    Completed,
    Empty
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
            value => Err(format!("Unknown value: {}", value))
        }
    }
}