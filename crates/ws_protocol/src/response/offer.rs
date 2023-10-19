use serde::{Deserialize, Serialize};

use crate::common::*;

/// Apparently, these are sent to a number of peers when they are set
/// in an AnnounceRequest
/// action = "announce"
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct MiddlemanOfferToPeer {
    pub action: AnnounceAction,
    /// Peer id of peer sending offer
    /// Note: if equal to client peer_id, client ignores offer
    pub peer_id: PeerId,
    pub info_hash: InfoHash,
    /// Gets copied from AnnounceRequestOffer
    pub offer: JsonValue,
    /// Gets copied from AnnounceRequestOffer
    pub offer_id: OfferId,
}
