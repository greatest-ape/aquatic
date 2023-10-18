use serde::{Deserialize, Serialize};

use crate::common::*;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum AnnounceEvent {
    Started,
    Stopped,
    Completed,
    Update,
}

impl Default for AnnounceEvent {
    fn default() -> Self {
        Self::Update
    }
}

/// Element of AnnounceRequest.offers
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AnnounceRequestOffer {
    pub offer: JsonValue,
    pub offer_id: OfferId,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AnnounceRequest {
    pub action: AnnounceAction,
    pub info_hash: InfoHash,
    pub peer_id: PeerId,
    /// Just called "left" in protocol. Is set to None in some cases, such as
    /// when opening a magnet link
    #[serde(rename = "left")]
    pub bytes_left: Option<usize>,
    /// Can be empty. Then, default is "update"
    #[serde(skip_serializing_if = "Option::is_none")]
    pub event: Option<AnnounceEvent>,

    /// Only when this is an array offers are sent to random peers
    /// Length of this is number of peers wanted?
    /// Max length of this is 10 in reference client code
    /// Not sent when announce event is stopped or completed
    pub offers: Option<Vec<AnnounceRequestOffer>>,
    /// Seems to only get sent by client when sending offers, and is also same
    /// as length of offers vector (or at least never less)
    /// Max length of this is 10 in reference client code
    /// Could probably be ignored, `offers.len()` should provide needed info
    pub numwant: Option<usize>,

    /// If empty, send response before sending offers (or possibly "skip sending update back"?)
    /// Else, send MiddlemanAnswerToPeer to peer with "to_peer_id" as peer_id.
    /// I think using Option is good, it seems like this isn't always set
    /// (same as `offers`)
    pub answer: Option<JsonValue>,
    /// Likely undefined if !(answer == true)
    pub to_peer_id: Option<PeerId>,
    /// Sent if answer is set
    pub offer_id: Option<OfferId>,
}
