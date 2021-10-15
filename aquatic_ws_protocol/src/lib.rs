pub mod common;
pub mod request;
pub mod response;

pub use common::*;
pub use request::*;
pub use response::*;

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
