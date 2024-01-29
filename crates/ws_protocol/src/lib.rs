//! WebTorrent protocol implementation
//!
//! Typical announce workflow:
//! - Peer A sends announce request with info hash and offers
//! - Tracker sends on offers to other peers announcing with that info hash and
//!   sends back announce response to peer A
//! - Tracker receives answers to those offers from other peers and send them
//!   on to peer A
//!
//! Typical scrape workflow
//! - Peer sends scrape request and receives scrape response

pub mod common;
pub mod incoming;
pub mod outgoing;

#[cfg(test)]
mod tests {
    use quickcheck::Arbitrary;
    use quickcheck_macros::quickcheck;

    use crate::{
        common::*,
        incoming::{
            AnnounceEvent, AnnounceRequest, AnnounceRequestOffer, InMessage, ScrapeRequest,
            ScrapeRequestInfoHashes,
        },
        outgoing::{
            AnnounceResponse, AnswerOutMessage, OfferOutMessage, OutMessage, ScrapeResponse,
            ScrapeStatistics,
        },
    };

    fn arbitrary_20_bytes(g: &mut quickcheck::Gen) -> [u8; 20] {
        let mut bytes = [0u8; 20];

        for byte in bytes.iter_mut() {
            *byte = u8::arbitrary(g);
        }

        bytes
    }

    fn rtc_offer() -> RtcOffer {
        RtcOffer {
            t: RtcOfferType::Offer,
            sdp: "test".into(),
        }
    }
    fn rtc_answer() -> RtcAnswer {
        RtcAnswer {
            t: RtcAnswerType::Answer,
            sdp: "test".into(),
        }
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

    impl Arbitrary for OfferOutMessage {
        fn arbitrary(g: &mut quickcheck::Gen) -> Self {
            Self {
                action: AnnounceAction::Announce,
                peer_id: Arbitrary::arbitrary(g),
                info_hash: Arbitrary::arbitrary(g),
                offer_id: Arbitrary::arbitrary(g),
                offer: rtc_offer(),
            }
        }
    }

    impl Arbitrary for AnswerOutMessage {
        fn arbitrary(g: &mut quickcheck::Gen) -> Self {
            Self {
                action: AnnounceAction::Announce,
                peer_id: Arbitrary::arbitrary(g),
                info_hash: Arbitrary::arbitrary(g),
                offer_id: Arbitrary::arbitrary(g),
                answer: rtc_answer(),
            }
        }
    }

    impl Arbitrary for AnnounceRequestOffer {
        fn arbitrary(g: &mut quickcheck::Gen) -> Self {
            Self {
                offer_id: Arbitrary::arbitrary(g),
                offer: rtc_offer(),
            }
        }
    }

    impl Arbitrary for AnnounceRequest {
        fn arbitrary(g: &mut quickcheck::Gen) -> Self {
            let has_offers_or_answer_or_neither: Option<bool> = Arbitrary::arbitrary(g);

            let mut offers: Option<Vec<AnnounceRequestOffer>> = None;
            let mut answer: Option<RtcAnswer> = None;
            let mut to_peer_id: Option<PeerId> = None;
            let mut offer_id: Option<OfferId> = None;

            match has_offers_or_answer_or_neither {
                Some(true) => {
                    offers = Some(Arbitrary::arbitrary(g));
                }
                Some(false) => {
                    answer = Some(rtc_answer());
                    to_peer_id = Some(Arbitrary::arbitrary(g));
                    offer_id = Some(Arbitrary::arbitrary(g));
                }
                None => (),
            }

            let numwant = offers.as_ref().map(|offers| offers.len());

            Self {
                action: AnnounceAction::Announce,
                info_hash: Arbitrary::arbitrary(g),
                peer_id: Arbitrary::arbitrary(g),
                bytes_left: Arbitrary::arbitrary(g),
                event: Arbitrary::arbitrary(g),
                offers,
                numwant,
                answer,
                answer_to_peer_id: to_peer_id,
                answer_offer_id: offer_id,
            }
        }
    }

    impl Arbitrary for AnnounceResponse {
        fn arbitrary(g: &mut quickcheck::Gen) -> Self {
            Self {
                action: AnnounceAction::Announce,
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
                action: ScrapeAction::Scrape,
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
                action: ScrapeAction::Scrape,
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
                (false, true) => Self::OfferOutMessage(Arbitrary::arbitrary(g)),
                (true, true) => Self::AnswerOutMessage(Arbitrary::arbitrary(g)),
            }
        }
    }

    #[cfg(feature = "tungstenite")]
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

    #[cfg(feature = "tungstenite")]
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

        arr.copy_from_slice(bytes);

        InfoHash(arr)
    }

    #[test]
    fn test_deserialize_info_hashes_vec() {
        let info_hashes = ScrapeRequestInfoHashes::Multiple(vec![
            info_hash_from_bytes(b"aaaabbbbccccddddeeee"),
            info_hash_from_bytes(b"aaaabbbbccccddddeeee"),
        ]);

        let expected = ScrapeRequest {
            action: ScrapeAction::Scrape,
            info_hashes: Some(info_hashes),
        };

        let observed: ScrapeRequest = unsafe {
            let mut input: String = r#"{
                "action": "scrape",
                "info_hash": ["aaaabbbbccccddddeeee", "aaaabbbbccccddddeeee"]
            }"#
            .into();

            ::simd_json::serde::from_str(&mut input).unwrap()
        };

        assert_eq!(expected, observed);
    }

    #[test]
    fn test_deserialize_info_hashes_str() {
        let info_hashes =
            ScrapeRequestInfoHashes::Single(info_hash_from_bytes(b"aaaabbbbccccddddeeee"));

        let expected = ScrapeRequest {
            action: ScrapeAction::Scrape,
            info_hashes: Some(info_hashes),
        };

        let observed: ScrapeRequest = unsafe {
            let mut input: String = r#"{
                "action": "scrape",
                "info_hash": "aaaabbbbccccddddeeee"
            }"#
            .into();

            ::simd_json::serde::from_str(&mut input).unwrap()
        };

        assert_eq!(expected, observed);
    }

    #[test]
    fn test_deserialize_info_hashes_null() {
        let observed: ScrapeRequest = unsafe {
            let mut input: String = r#"{
                "action": "scrape",
                "info_hash": null
            }"#
            .into();

            ::simd_json::serde::from_str(&mut input).unwrap()
        };

        let expected = ScrapeRequest {
            action: ScrapeAction::Scrape,
            info_hashes: None,
        };

        assert_eq!(expected, observed);
    }

    #[test]
    fn test_deserialize_info_hashes_missing() {
        let observed: ScrapeRequest = unsafe {
            let mut input: String = r#"{
                "action": "scrape"
            }"#
            .into();

            ::simd_json::serde::from_str(&mut input).unwrap()
        };

        let expected = ScrapeRequest {
            action: ScrapeAction::Scrape,
            info_hashes: None,
        };

        assert_eq!(expected, observed);
    }

    #[quickcheck]
    fn quickcheck_serde_identity_info_hashes(info_hashes: ScrapeRequestInfoHashes) -> bool {
        let deserialized: ScrapeRequestInfoHashes = unsafe {
            let mut json = ::simd_json::serde::to_string(&info_hashes).unwrap();

            ::simd_json::serde::from_str(&mut json).unwrap()
        };

        info_hashes == deserialized
    }
}
