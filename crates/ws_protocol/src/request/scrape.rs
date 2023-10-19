use serde::{Deserialize, Serialize};

use crate::common::*;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(untagged)]
pub enum ScrapeRequestInfoHashes {
    Single(InfoHash),
    Multiple(Vec<InfoHash>),
}

impl ScrapeRequestInfoHashes {
    pub fn as_vec(self) -> Vec<InfoHash> {
        match self {
            Self::Single(info_hash) => vec![info_hash],
            Self::Multiple(info_hashes) => info_hashes,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ScrapeRequest {
    pub action: ScrapeAction,
    // If omitted, scrape for all torrents, apparently
    // There is some kind of parsing here too which accepts a single info hash
    // and puts it into a vector
    #[serde(rename = "info_hash")]
    pub info_hashes: Option<ScrapeRequestInfoHashes>,
}
