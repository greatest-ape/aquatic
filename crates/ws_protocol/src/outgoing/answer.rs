use serde::{Deserialize, Serialize};

use crate::common::*;

/// Message sent to peer when other peer has replied to its WebRTC offer
///
/// Sent if fields answer, to_peer_id and offer_id are set in announce request
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AnswerOutMessage {
    /// Always "announce"
    pub action: AnnounceAction,
    /// Note: if equal to client peer_id, client ignores answer
    pub peer_id: PeerId,
    pub info_hash: InfoHash,
    pub answer: RtcAnswer,
    pub offer_id: OfferId,
}
