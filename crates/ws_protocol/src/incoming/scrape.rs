use serde::{Deserialize, Serialize};

use crate::common::*;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ScrapeRequest {
    /// Always "scrape"
    pub action: ScrapeAction,
    /// Info hash or info hashes
    ///
    /// Notes from reference implementation:
    /// - If omitted, scrape for all torrents, apparently
    /// - Accepts a single info hash or an array of info hashes
    #[serde(rename = "info_hash")]
    pub info_hashes: Option<ScrapeRequestInfoHashes>,
}

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
