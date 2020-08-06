use hashbrown::HashMap;
use serde::{Serialize, Deserialize};

mod serde_helpers;

use serde_helpers::*;


#[derive(Debug, Clone, Copy, Hash, PartialEq, Eq, Serialize, Deserialize)]
#[serde(transparent)]
pub struct PeerId(
    #[serde(
        deserialize_with = "deserialize_20_bytes",
        serialize_with = "serialize_20_bytes"
    )]
    pub [u8; 20]
);


#[derive(Debug, Clone, Copy, Hash, PartialEq, Eq, Serialize, Deserialize)]
#[serde(transparent)]
pub struct InfoHash(
    #[serde(
        deserialize_with = "deserialize_20_bytes",
        serialize_with = "serialize_20_bytes"
    )]
    pub [u8; 20]
);


#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(transparent)]
pub struct OfferId(
    #[serde(
        deserialize_with = "deserialize_20_bytes",
        serialize_with = "serialize_20_bytes"
    )]
    pub [u8; 20]
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
    Update
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
    pub info_hash: InfoHash,
    /// Client checks if this is null, not clear why
    pub complete: usize,
    pub incomplete: usize,
    #[serde(rename = "interval")]
    pub announce_interval: usize, // Default 2 min probably
}


#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ScrapeRequest {
    // If omitted, scrape for all torrents, apparently
    // There is some kind of parsing here too which accepts a single info hash
    // and puts it into a vector
    #[serde(
        rename = "info_hash",
        deserialize_with = "deserialize_info_hashes",
        default
    )]
    pub info_hashes: Vec<InfoHash>,
}


#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ScrapeStatistics {
    pub complete: usize,
    pub incomplete: usize,
    pub downloaded: usize,
}


#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ScrapeResponse {
    pub files: HashMap<InfoHash, ScrapeStatistics>,
    // Looks like `flags` field is ignored in reference client
    // pub flags: HashMap<String, usize>,
}


#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Action {
    Announce,
    Scrape
}


/// Helper for serializing and deserializing messages
#[derive(Debug, Clone, Serialize, Deserialize)]
struct ActionWrapper<T> {
    pub action: Action,
    #[serde(flatten)]
    pub inner: T,
}


impl <T>ActionWrapper<T> {
    #[inline]
    pub fn announce(t: T) -> Self {
        Self {
            action: Action::Announce,
            inner: t
        }
    }
    #[inline]
    pub fn scrape(t: T) -> Self {
        Self {
            action: Action::Scrape,
            inner: t
        }
    }
}


#[derive(Debug, Clone, PartialEq, Eq)]
pub enum InMessage {
    AnnounceRequest(AnnounceRequest),
    ScrapeRequest(ScrapeRequest),
}


impl InMessage {
    /// Try parsing as announce request first. If that fails, try parsing as
    /// scrape request, or return None
    #[inline]
    pub fn from_ws_message(ws_message: tungstenite::Message) -> Option<Self> {
        use tungstenite::Message::{Text, Binary};

        let text = match ws_message {
            Text(text) => Some(text),
            Binary(bytes) => String::from_utf8(bytes).ok(),
            _ => None
        }?;

        let res: Result<ActionWrapper<AnnounceRequest>, _> = serde_json::from_str(&text);

        if let Ok(ActionWrapper { action: Action::Announce, inner }) = res {
            return Some(InMessage::AnnounceRequest(inner));
        }
        
        let res: Result<ActionWrapper<ScrapeRequest>, _> = serde_json::from_str(&text);

        if let Ok(ActionWrapper { action: Action::Scrape, inner }) = res {
            return Some(InMessage::ScrapeRequest(inner));
        }

        None
    }

    pub fn to_ws_message(&self) -> ::tungstenite::Message {
        let text = match self {
            InMessage::AnnounceRequest(r) => {
                serde_json::to_string(&ActionWrapper::announce(r)).unwrap()
            },
            InMessage::ScrapeRequest(r) => {
                serde_json::to_string(&ActionWrapper::scrape(r)).unwrap()
            },
        };

        ::tungstenite::Message::from(text)
    }
}


#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(untagged)]
pub enum OutMessage {
    AnnounceResponse(AnnounceResponse),
    ScrapeResponse(ScrapeResponse),
    Offer(MiddlemanOfferToPeer),
    Answer(MiddlemanAnswerToPeer),
}


impl OutMessage {
    #[inline]
    pub fn into_ws_message(self) -> tungstenite::Message {
        let json = match self {
            Self::AnnounceResponse(message) => {
                serde_json::to_string(
                    &ActionWrapper::announce(message)
                ).unwrap()
            },
            Self::Offer(message) => {
                serde_json::to_string(
                    &ActionWrapper::announce(message)
                ).unwrap()
            },
            Self::Answer(message) => {
                serde_json::to_string(
                    &ActionWrapper::announce(message)
                ).unwrap()
            },
            Self::ScrapeResponse(message) => {
                serde_json::to_string(
                    &ActionWrapper::scrape(message)
                ).unwrap()
            },
        };
                
        tungstenite::Message::from(json)
    }

    #[inline]
    pub fn from_ws_message(
        message: ::tungstenite::Message
    ) -> ::anyhow::Result<Self> {
        use tungstenite::Message::{Text, Binary};

        let text = match message {
            Text(text) => text,
            Binary(bytes) => String::from_utf8(bytes)?,
            _ => return Err(anyhow::anyhow!("message type not supported")),
        };

        if text.contains("answer"){
            Ok(Self::Answer(::serde_json::from_str(&text)?))
        } else if text.contains("offer"){
            Ok(Self::Offer(::serde_json::from_str(&text)?))
        } else if text.contains("interval"){
            Ok(Self::AnnounceResponse(::serde_json::from_str(&text)?))
        } else if text.contains("scrape"){
            Ok(Self::ScrapeResponse(::serde_json::from_str(&text)?))
        } else {
            Err(anyhow::anyhow!("Could not determine response type"))
        }
    }
}


#[cfg(test)]
mod tests {
    use quickcheck::Arbitrary;
    use quickcheck_macros::quickcheck;

    use super::*;

    fn arbitrary_20_bytes<G: quickcheck::Gen>(g: &mut G) -> [u8; 20] {
        let mut bytes = [0u8; 20];

        for byte in bytes.iter_mut() {
            *byte = u8::arbitrary(g);
        }

        bytes
    }

    impl Arbitrary for InfoHash {
        fn arbitrary<G: quickcheck::Gen>(g: &mut G) -> Self {
            Self(arbitrary_20_bytes(g))
        }
    }

    impl Arbitrary for PeerId {
        fn arbitrary<G: quickcheck::Gen>(g: &mut G) -> Self {
            Self(arbitrary_20_bytes(g))
        }
    }

    impl Arbitrary for OfferId {
        fn arbitrary<G: quickcheck::Gen>(g: &mut G) -> Self {
            Self(arbitrary_20_bytes(g))
        }
    }

    impl Arbitrary for JsonValue {
        fn arbitrary<G: quickcheck::Gen>(g: &mut G) -> Self {
            Self(::serde_json::json!(r#"{ "sdp": "test" }"#))
        }
    }

    impl Arbitrary for AnnounceEvent {
        fn arbitrary<G: quickcheck::Gen>(g: &mut G) -> Self {
            match (bool::arbitrary(g), bool::arbitrary(g)){
                (false, false) => Self::Started,
                (true, false) => Self::Started,
                (false, true) => Self::Completed,
                (true, true) => Self::Update,
            }
        }
    }

    impl Arbitrary for MiddlemanOfferToPeer {
        fn arbitrary<G: quickcheck::Gen>(g: &mut G) -> Self {
            Self {
                peer_id: Arbitrary::arbitrary(g),
                info_hash: Arbitrary::arbitrary(g),
                offer_id: Arbitrary::arbitrary(g),
                offer: Arbitrary::arbitrary(g)
            }
        }
    }

    impl Arbitrary for MiddlemanAnswerToPeer {
        fn arbitrary<G: quickcheck::Gen>(g: &mut G) -> Self {
            Self {
                peer_id: Arbitrary::arbitrary(g),
                info_hash: Arbitrary::arbitrary(g),
                offer_id: Arbitrary::arbitrary(g),
                answer: Arbitrary::arbitrary(g)
            }
        }
    }

    impl Arbitrary for AnnounceRequestOffer {
        fn arbitrary<G: quickcheck::Gen>(g: &mut G) -> Self {
            Self {
                offer_id: Arbitrary::arbitrary(g),
                offer: Arbitrary::arbitrary(g)
            }
        }
    }

    impl Arbitrary for AnnounceRequest {
        fn arbitrary<G: quickcheck::Gen>(g: &mut G) -> Self {
            let has_offers_or_answer_or_neither: Option<bool> = Arbitrary::arbitrary(g);

            let mut offers: Option<Vec<AnnounceRequestOffer>> = None;
            let mut answer: Option<JsonValue> = None;
            let mut to_peer_id: Option<PeerId> = None;
            let mut offer_id: Option<OfferId> = None;

            match has_offers_or_answer_or_neither {
                Some(true) => {
                    offers = Some(Arbitrary::arbitrary(g));
                },
                Some(false) => {
                    answer = Some(Arbitrary::arbitrary(g));
                    to_peer_id = Some(Arbitrary::arbitrary(g));
                    offer_id = Some(Arbitrary::arbitrary(g));
                },
                None => (),
            }

            let numwant = offers.as_ref()
                .map(|offers| offers.len());

            Self {
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
        fn arbitrary<G: quickcheck::Gen>(g: &mut G) -> Self {
            Self {
                info_hash: Arbitrary::arbitrary(g),
                complete: Arbitrary::arbitrary(g),
                incomplete: Arbitrary::arbitrary(g),
                announce_interval: Arbitrary::arbitrary(g),
            }
        }
    }

    impl Arbitrary for ScrapeRequest {
        fn arbitrary<G: quickcheck::Gen>(g: &mut G) -> Self {
            Self {
                info_hashes: Arbitrary::arbitrary(g),
            }
        }
    }

    impl Arbitrary for ScrapeStatistics {
        fn arbitrary<G: quickcheck::Gen>(g: &mut G) -> Self {
            Self {
                complete: Arbitrary::arbitrary(g),
                incomplete: Arbitrary::arbitrary(g),
                downloaded: Arbitrary::arbitrary(g),
            }
        }
    }

    impl Arbitrary for ScrapeResponse {
        fn arbitrary<G: quickcheck::Gen>(g: &mut G) -> Self {
            let files: Vec<(InfoHash, ScrapeStatistics)> = Arbitrary::arbitrary(g);

            Self {
                files: files.into_iter().collect(),
            }
        }
    }

    impl Arbitrary for InMessage {
        fn arbitrary<G: quickcheck::Gen>(g: &mut G) -> Self {
            if Arbitrary::arbitrary(g){
                Self::AnnounceRequest(Arbitrary::arbitrary(g))
            } else {
                Self::ScrapeRequest(Arbitrary::arbitrary(g))
            }
        }
    }

    impl Arbitrary for OutMessage {
        fn arbitrary<G: quickcheck::Gen>(g: &mut G) -> Self {
            match (Arbitrary::arbitrary(g), Arbitrary::arbitrary(g)){
                (false, false) => Self::AnnounceResponse(Arbitrary::arbitrary(g)),
                (true, false) => Self::ScrapeResponse(Arbitrary::arbitrary(g)),
                (false, true) => Self::Offer(Arbitrary::arbitrary(g)),
                (true, true) => Self::Answer(Arbitrary::arbitrary(g)),
            }
        }
    }

    #[quickcheck]
    fn test_serialize_deserialize_in_message(in_message_1: InMessage){
        dbg!(in_message_1.clone());

        let ws_message = in_message_1.to_ws_message();
        dbg!(ws_message.clone());

        let in_message_2 = InMessage::from_ws_message(ws_message).unwrap();
        dbg!(in_message_2.clone());

        assert_eq!(in_message_1, in_message_2);
    }

    #[quickcheck]
    fn test_serialize_deserialize_out_message(out_message_1: OutMessage){
        dbg!(out_message_1.clone());

        let ws_message = out_message_1.clone().into_ws_message();
        dbg!(ws_message.clone());

        let out_message_2 = OutMessage::from_ws_message(ws_message).unwrap();
        dbg!(out_message_2.clone());

        assert_eq!(out_message_1, out_message_2);
    }
}