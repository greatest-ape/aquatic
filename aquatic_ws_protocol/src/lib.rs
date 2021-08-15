use anyhow::Context;
use hashbrown::HashMap;
use serde::{Deserialize, Deserializer, Serialize, Serializer};

mod serde_helpers;

use serde_helpers::*;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct AnnounceAction;

impl Serialize for AnnounceAction {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        serializer.serialize_str("announce")
    }
}

impl<'de> Deserialize<'de> for AnnounceAction {
    fn deserialize<D>(deserializer: D) -> Result<AnnounceAction, D::Error>
    where
        D: Deserializer<'de>,
    {
        deserializer.deserialize_str(AnnounceActionVisitor)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ScrapeAction;

impl Serialize for ScrapeAction {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        serializer.serialize_str("scrape")
    }
}

impl<'de> Deserialize<'de> for ScrapeAction {
    fn deserialize<D>(deserializer: D) -> Result<ScrapeAction, D::Error>
    where
        D: Deserializer<'de>,
    {
        deserializer.deserialize_str(ScrapeActionVisitor)
    }
}

#[derive(Debug, Clone, Copy, Hash, PartialEq, Eq, Serialize, Deserialize)]
#[serde(transparent)]
pub struct PeerId(
    #[serde(
        deserialize_with = "deserialize_20_bytes",
        serialize_with = "serialize_20_bytes"
    )]
    pub [u8; 20],
);

#[derive(Debug, Clone, Copy, Hash, PartialEq, Eq, Serialize, Deserialize)]
#[serde(transparent)]
pub struct InfoHash(
    #[serde(
        deserialize_with = "deserialize_20_bytes",
        serialize_with = "serialize_20_bytes"
    )]
    pub [u8; 20],
);

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(transparent)]
pub struct OfferId(
    #[serde(
        deserialize_with = "deserialize_20_bytes",
        serialize_with = "serialize_20_bytes"
    )]
    pub [u8; 20],
);

/// Some kind of nested structure from https://www.npmjs.com/package/simple-peer
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(transparent)]
pub struct JsonValue(pub ::serde_json::Value);

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

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ScrapeRequest {
    pub action: ScrapeAction,
    // If omitted, scrape for all torrents, apparently
    // There is some kind of parsing here too which accepts a single info hash
    // and puts it into a vector
    #[serde(rename = "info_hash")]
    pub info_hashes: Option<ScrapeRequestInfoHashes>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ScrapeStatistics {
    pub complete: usize,
    pub incomplete: usize,
    pub downloaded: usize,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ScrapeResponse {
    pub action: ScrapeAction,
    pub files: HashMap<InfoHash, ScrapeStatistics>,
    // Looks like `flags` field is ignored in reference client
    // pub flags: HashMap<String, usize>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(untagged)]
pub enum InMessage {
    AnnounceRequest(AnnounceRequest),
    ScrapeRequest(ScrapeRequest),
}

impl InMessage {
    #[inline]
    pub fn to_ws_message(&self) -> ::tungstenite::Message {
        ::tungstenite::Message::from(::serde_json::to_string(&self).unwrap())
    }

    #[inline]
    pub fn from_ws_message(ws_message: tungstenite::Message) -> ::anyhow::Result<Self> {
        use tungstenite::Message::Text;

        let mut text = if let Text(text) = ws_message {
            text
        } else {
            return Err(anyhow::anyhow!("Message is not text"));
        };

        return ::simd_json::serde::from_str(&mut text).context("deserialize with serde");
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(untagged)]
pub enum OutMessage {
    Offer(MiddlemanOfferToPeer),
    Answer(MiddlemanAnswerToPeer),
    AnnounceResponse(AnnounceResponse),
    ScrapeResponse(ScrapeResponse),
}

impl OutMessage {
    #[inline]
    pub fn to_ws_message(&self) -> tungstenite::Message {
        ::tungstenite::Message::from(::serde_json::to_string(&self).unwrap())
    }

    #[inline]
    pub fn from_ws_message(message: ::tungstenite::Message) -> ::anyhow::Result<Self> {
        use tungstenite::Message::{Binary, Text};

        let mut text = match message {
            Text(text) => text,
            Binary(bytes) => String::from_utf8(bytes)?,
            _ => return Err(anyhow::anyhow!("Message is neither text nor bytes")),
        };

        Ok(::simd_json::serde::from_str(&mut text)?)
    }
}

#[cfg(test)]
mod tests {
    use quickcheck::Arbitrary;
    use quickcheck_macros::quickcheck;

    use super::*;

    fn arbitrary_20_bytes(g: &mut quickcheck::Gen) -> [u8; 20] {
        let mut bytes = [0u8; 20];

        for byte in bytes.iter_mut() {
            *byte = u8::arbitrary(g);
        }

        bytes
    }

    fn sdp_json_value() -> JsonValue {
        JsonValue(::serde_json::json!({ "sdp": "test" }))
    }

    impl Arbitrary for InfoHash {
        fn arbitrary(g: &mut quickcheck::Gen) -> Self {
            Self(arbitrary_20_bytes(g))
        }
    }

    impl Arbitrary for PeerId {
        fn arbitrary(g: &mut quickcheck::Gen) -> Self {
            Self(arbitrary_20_bytes(g))
        }
    }

    impl Arbitrary for OfferId {
        fn arbitrary(g: &mut quickcheck::Gen) -> Self {
            Self(arbitrary_20_bytes(g))
        }
    }

    impl Arbitrary for AnnounceEvent {
        fn arbitrary(g: &mut quickcheck::Gen) -> Self {
            match (bool::arbitrary(g), bool::arbitrary(g)) {
                (false, false) => Self::Started,
                (true, false) => Self::Started,
                (false, true) => Self::Completed,
                (true, true) => Self::Update,
            }
        }
    }

    impl Arbitrary for MiddlemanOfferToPeer {
        fn arbitrary(g: &mut quickcheck::Gen) -> Self {
            Self {
                action: AnnounceAction,
                peer_id: Arbitrary::arbitrary(g),
                info_hash: Arbitrary::arbitrary(g),
                offer_id: Arbitrary::arbitrary(g),
                offer: sdp_json_value(),
            }
        }
    }

    impl Arbitrary for MiddlemanAnswerToPeer {
        fn arbitrary(g: &mut quickcheck::Gen) -> Self {
            Self {
                action: AnnounceAction,
                peer_id: Arbitrary::arbitrary(g),
                info_hash: Arbitrary::arbitrary(g),
                offer_id: Arbitrary::arbitrary(g),
                answer: sdp_json_value(),
            }
        }
    }

    impl Arbitrary for AnnounceRequestOffer {
        fn arbitrary(g: &mut quickcheck::Gen) -> Self {
            Self {
                offer_id: Arbitrary::arbitrary(g),
                offer: sdp_json_value(),
            }
        }
    }

    impl Arbitrary for AnnounceRequest {
        fn arbitrary(g: &mut quickcheck::Gen) -> Self {
            let has_offers_or_answer_or_neither: Option<bool> = Arbitrary::arbitrary(g);

            let mut offers: Option<Vec<AnnounceRequestOffer>> = None;
            let mut answer: Option<JsonValue> = None;
            let mut to_peer_id: Option<PeerId> = None;
            let mut offer_id: Option<OfferId> = None;

            match has_offers_or_answer_or_neither {
                Some(true) => {
                    offers = Some(Arbitrary::arbitrary(g));
                }
                Some(false) => {
                    answer = Some(sdp_json_value());
                    to_peer_id = Some(Arbitrary::arbitrary(g));
                    offer_id = Some(Arbitrary::arbitrary(g));
                }
                None => (),
            }

            let numwant = offers.as_ref().map(|offers| offers.len());

            Self {
                action: AnnounceAction,
                info_hash: Arbitrary::arbitrary(g),
                peer_id: Arbitrary::arbitrary(g),
                bytes_left: Arbitrary::arbitrary(g),
                event: Arbitrary::arbitrary(g),
                offers,
                numwant,
                answer,
                to_peer_id,
                offer_id,
            }
        }
    }

    impl Arbitrary for AnnounceResponse {
        fn arbitrary(g: &mut quickcheck::Gen) -> Self {
            Self {
                action: AnnounceAction,
                info_hash: Arbitrary::arbitrary(g),
                complete: Arbitrary::arbitrary(g),
                incomplete: Arbitrary::arbitrary(g),
                announce_interval: Arbitrary::arbitrary(g),
            }
        }
    }

    impl Arbitrary for ScrapeRequest {
        fn arbitrary(g: &mut quickcheck::Gen) -> Self {
            Self {
                action: ScrapeAction,
                info_hashes: Arbitrary::arbitrary(g),
            }
        }
    }

    impl Arbitrary for ScrapeRequestInfoHashes {
        fn arbitrary(g: &mut quickcheck::Gen) -> Self {
            if Arbitrary::arbitrary(g) {
                ScrapeRequestInfoHashes::Multiple(Arbitrary::arbitrary(g))
            } else {
                ScrapeRequestInfoHashes::Single(Arbitrary::arbitrary(g))
            }
        }
    }

    impl Arbitrary for ScrapeStatistics {
        fn arbitrary(g: &mut quickcheck::Gen) -> Self {
            Self {
                complete: Arbitrary::arbitrary(g),
                incomplete: Arbitrary::arbitrary(g),
                downloaded: Arbitrary::arbitrary(g),
            }
        }
    }

    impl Arbitrary for ScrapeResponse {
        fn arbitrary(g: &mut quickcheck::Gen) -> Self {
            let files: Vec<(InfoHash, ScrapeStatistics)> = Arbitrary::arbitrary(g);

            Self {
                action: ScrapeAction,
                files: files.into_iter().collect(),
            }
        }
    }

    impl Arbitrary for InMessage {
        fn arbitrary(g: &mut quickcheck::Gen) -> Self {
            if Arbitrary::arbitrary(g) {
                Self::AnnounceRequest(Arbitrary::arbitrary(g))
            } else {
                Self::ScrapeRequest(Arbitrary::arbitrary(g))
            }
        }
    }

    impl Arbitrary for OutMessage {
        fn arbitrary(g: &mut quickcheck::Gen) -> Self {
            match (Arbitrary::arbitrary(g), Arbitrary::arbitrary(g)) {
                (false, false) => Self::AnnounceResponse(Arbitrary::arbitrary(g)),
                (true, false) => Self::ScrapeResponse(Arbitrary::arbitrary(g)),
                (false, true) => Self::Offer(Arbitrary::arbitrary(g)),
                (true, true) => Self::Answer(Arbitrary::arbitrary(g)),
            }
        }
    }

    #[quickcheck]
    fn quickcheck_serde_identity_in_message(in_message_1: InMessage) -> bool {
        let ws_message = in_message_1.to_ws_message();

        let in_message_2 = InMessage::from_ws_message(ws_message.clone()).unwrap();

        let success = in_message_1 == in_message_2;

        if !success {
            dbg!(in_message_1);
            dbg!(in_message_2);
            if let ::tungstenite::Message::Text(text) = ws_message {
                println!("{}", text);
            }
        }

        success
    }

    #[quickcheck]
    fn quickcheck_serde_identity_out_message(out_message_1: OutMessage) -> bool {
        let ws_message = out_message_1.to_ws_message();

        let out_message_2 = OutMessage::from_ws_message(ws_message.clone()).unwrap();

        let success = out_message_1 == out_message_2;

        if !success {
            dbg!(out_message_1);
            dbg!(out_message_2);
            if let ::tungstenite::Message::Text(text) = ws_message {
                println!("{}", text);
            }
        }

        success
    }

    fn info_hash_from_bytes(bytes: &[u8]) -> InfoHash {
        let mut arr = [0u8; 20];

        assert!(bytes.len() == 20);

        arr.copy_from_slice(&bytes[..]);

        InfoHash(arr)
    }

    #[test]
    fn test_deserialize_info_hashes_vec() {
        let mut input: String = r#"{
            "action": "scrape",
            "info_hash": ["aaaabbbbccccddddeeee", "aaaabbbbccccddddeeee"]
        }"#
        .into();

        let info_hashes = ScrapeRequestInfoHashes::Multiple(vec![
            info_hash_from_bytes(b"aaaabbbbccccddddeeee"),
            info_hash_from_bytes(b"aaaabbbbccccddddeeee"),
        ]);

        let expected = ScrapeRequest {
            action: ScrapeAction,
            info_hashes: Some(info_hashes),
        };

        let observed: ScrapeRequest = ::simd_json::serde::from_str(&mut input).unwrap();

        assert_eq!(expected, observed);
    }

    #[test]
    fn test_deserialize_info_hashes_str() {
        let mut input: String = r#"{
            "action": "scrape",
            "info_hash": "aaaabbbbccccddddeeee"
        }"#
        .into();

        let info_hashes =
            ScrapeRequestInfoHashes::Single(info_hash_from_bytes(b"aaaabbbbccccddddeeee"));

        let expected = ScrapeRequest {
            action: ScrapeAction,
            info_hashes: Some(info_hashes),
        };

        let observed: ScrapeRequest = ::simd_json::serde::from_str(&mut input).unwrap();

        assert_eq!(expected, observed);
    }

    #[test]
    fn test_deserialize_info_hashes_null() {
        let mut input: String = r#"{
            "action": "scrape",
            "info_hash": null
        }"#
        .into();

        let expected = ScrapeRequest {
            action: ScrapeAction,
            info_hashes: None,
        };

        let observed: ScrapeRequest = ::simd_json::serde::from_str(&mut input).unwrap();

        assert_eq!(expected, observed);
    }

    #[test]
    fn test_deserialize_info_hashes_missing() {
        let mut input: String = r#"{
            "action": "scrape"
        }"#
        .into();

        let expected = ScrapeRequest {
            action: ScrapeAction,
            info_hashes: None,
        };

        let observed: ScrapeRequest = ::simd_json::serde::from_str(&mut input).unwrap();

        assert_eq!(expected, observed);
    }

    #[quickcheck]
    fn quickcheck_serde_identity_info_hashes(info_hashes: ScrapeRequestInfoHashes) -> bool {
        let mut json = ::simd_json::serde::to_string(&info_hashes).unwrap();

        println!("{}", json);

        let deserialized: ScrapeRequestInfoHashes =
            ::simd_json::serde::from_str(&mut json).unwrap();

        let success = info_hashes == deserialized;

        if !success {}

        success
    }
}
