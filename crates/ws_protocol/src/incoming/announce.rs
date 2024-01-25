use serde::{Deserialize, Serialize};

use crate::common::*;

/// Announce request
///
/// Can optionally contain:
/// - A number of WebRTC offers to be sent on to other peers. In this case,
///   fields 'offers' and 'numwant' are set
/// - An answer to a WebRTC offer from another peer. In this case, fields
///   'answer', 'to_peer_id' and 'offer_id' are set.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AnnounceRequest {
    /// Always "announce"
    pub action: AnnounceAction,
    pub info_hash: InfoHash,
    pub peer_id: PeerId,
    /// Bytes left
    ///
    /// Just called "left" in protocol. Is set to None in some cases, such as
    /// when opening a magnet link
    #[serde(rename = "left")]
    pub bytes_left: Option<usize>,
    /// Can be empty. Then, default is "update"
    #[serde(skip_serializing_if = "Option::is_none")]
    pub event: Option<AnnounceEvent>,

    /// WebRTC offers (with offer id's) that peer wants sent on to random other peers
    ///
    /// Notes from reference implementation:
    /// - Only when this is an array offers are sent to other peers
    /// - Length of this is number of peers wanted?
    /// - Max length of this is 10 in reference client code
    /// - Not sent when announce event is stopped or completed
    pub offers: Option<Vec<AnnounceRequestOffer>>,
    /// Number of peers wanted
    ///
    /// Notes from reference implementation:
    /// - Seems to only get sent by client when sending offers, and is also
    ///   same as length of offers vector (or at least never smaller)
    /// - Max length of this is 10 in reference client code
    /// - Could probably be ignored, `offers.len()` should provide needed info
    pub numwant: Option<usize>,

    /// WebRTC answer to previous offer from other peer, to be passed on to it
    ///
    /// Notes from reference implementation:
    /// - If empty, send response before sending offers (or possibly "skip
    ///   sending update back"?)
    /// - Else, send AnswerOutMessage to peer with "to_peer_id" as peer_id
    pub answer: Option<RtcAnswer>,
    /// Which peer to send answer to
    #[serde(rename = "to_peer_id")]
    pub answer_to_peer_id: Option<PeerId>,
    /// OfferID of offer this is an answer to
    #[serde(rename = "offer_id")]
    pub answer_offer_id: Option<OfferId>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
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
    pub offer: RtcOffer,
    pub offer_id: OfferId,
}
