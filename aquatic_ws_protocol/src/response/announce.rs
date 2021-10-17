use serde::{Deserialize, Serialize};

use crate::common::*;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AnnounceResponse {
    pub action: AnnounceAction,
    pub info_hash: InfoHash,
    /// Client checks if this is null, not clear why
    pub complete: usize,
    pub incomplete: usize,
    #[serde(rename = "interval")]
    pub announce_interval: usize, // Default 2 min probably
}
