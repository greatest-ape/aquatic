use std::borrow::Cow;

use serde::{Deserialize, Serialize};

use crate::common::*;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ErrorResponse {
    #[serde(rename = "failure reason")]
    pub failure_reason: Cow<'static, str>,
    /// Action of original request
    #[serde(skip_serializing_if = "Option::is_none")]
    pub action: Option<ErrorResponseAction>,
    // Should not be renamed
    #[serde(skip_serializing_if = "Option::is_none")]
    pub info_hash: Option<InfoHash>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ErrorResponseAction {
    Announce,
    Scrape,
}
