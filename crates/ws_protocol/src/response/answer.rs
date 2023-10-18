use serde::{Deserialize, Serialize};

use crate::common::*;

/// If announce request has answer = true, send this to peer with
/// peer id == "to_peer_id" field
/// Action field should be 'announce'
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct MiddlemanAnswerToPeer {
    pub action: AnnounceAction,
    /// Note: if equal to client peer_id, client ignores answer
    pub peer_id: PeerId,
    pub info_hash: InfoHash,
    pub answer: JsonValue,
    pub offer_id: OfferId,
}
