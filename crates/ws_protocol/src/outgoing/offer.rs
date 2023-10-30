use serde::{Deserialize, Serialize};

use crate::common::*;

/// Message sent to peer when other peer wants to initiate a WebRTC connection
///
/// One is sent for each offer in an announce request
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct OfferOutMessage {
    /// Always "announce"
    pub action: AnnounceAction,
    /// Peer id of peer sending offer
    ///
    /// Note: if equal to client peer_id, reference client ignores offer
    pub peer_id: PeerId,
    /// Torrent info hash
    pub info_hash: InfoHash,
    /// Gets copied from AnnounceRequestOffer
    pub offer: RtcOffer,
    /// Gets copied from AnnounceRequestOffer
    pub offer_id: OfferId,
}
