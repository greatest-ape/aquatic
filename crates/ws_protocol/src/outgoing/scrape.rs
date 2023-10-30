use hashbrown::HashMap;
use serde::{Deserialize, Serialize};

use crate::common::*;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ScrapeResponse {
    pub action: ScrapeAction,
    pub files: HashMap<InfoHash, ScrapeStatistics>,
    // It looks like `flags` field is ignored in reference client
    // pub flags: HashMap<String, usize>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ScrapeStatistics {
    pub complete: usize,
    pub incomplete: usize,
    pub downloaded: usize,
}
