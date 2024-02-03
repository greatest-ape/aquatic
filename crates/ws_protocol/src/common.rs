use serde::{de::Visitor, Deserialize, Deserializer, Serialize, Serializer};

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

#[derive(Debug, Clone, Copy, Hash, PartialEq, Eq, Serialize, Deserialize)]
#[serde(transparent)]
pub struct OfferId(
    #[serde(
        deserialize_with = "deserialize_20_bytes",
        serialize_with = "serialize_20_bytes"
    )]
    pub [u8; 20],
);

/// Serializes to and deserializes from "announce"
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum AnnounceAction {
    Announce,
}

/// Serializes to and deserializes from "scrape"
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ScrapeAction {
    Scrape,
}

/// Serializes to and deserializes from "offer"
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum RtcOfferType {
    Offer,
}

/// Serializes to and deserializes from "answer"
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum RtcAnswerType {
    Answer,
}

/// Nested structure with SDP offer from https://www.npmjs.com/package/simple-peer
///
/// Created using https://developer.mozilla.org/en-US/docs/Web/API/RTCPeerConnection/createOffer
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RtcOffer {
    /// Always "offer"
    #[serde(rename = "type")]
    pub t: RtcOfferType,
    pub sdp: String,
}

/// Nested structure with SDP answer from https://www.npmjs.com/package/simple-peer
///
/// Created using https://developer.mozilla.org/en-US/docs/Web/API/RTCPeerConnection/createAnswer
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RtcAnswer {
    /// Always "answer"
    #[serde(rename = "type")]
    pub t: RtcAnswerType,
    pub sdp: String,
}

fn serialize_20_bytes<S>(data: &[u8; 20], serializer: S) -> Result<S::Ok, S::Error>
where
    S: Serializer,
{
    // Length of 40 is enough since each char created from a byte will
    // utf-8-encode to max 2 bytes
    let mut str_buffer = [0u8; 40];
    let mut offset = 0;

    for byte in data {
        offset += char::from(*byte)
            .encode_utf8(&mut str_buffer[offset..])
            .len();
    }

    let text = ::std::str::from_utf8(&str_buffer[..offset]).unwrap();

    serializer.serialize_str(text)
}

struct TwentyByteVisitor;

impl<'de> Visitor<'de> for TwentyByteVisitor {
    type Value = [u8; 20];

    fn expecting(&self, formatter: &mut std::fmt::Formatter) -> std::fmt::Result {
        formatter.write_str("string consisting of 20 bytes")
    }

    #[inline]
    fn visit_str<E>(self, value: &str) -> Result<Self::Value, E>
    where
        E: ::serde::de::Error,
    {
        // Value is encoded in nodejs reference client something as follows:
        // ```
        // var infoHash = 'abcd..'; // 40 hexadecimals
        // Buffer.from(infoHash, 'hex').toString('binary');
        // ```
        // As I understand it:
        // - the code above produces a UTF16 string of 20 chars, each having
        //   only the "low byte" set (e.g., numeric value ranges from 0-255)
        // - serde_json decodes this to string of 20 chars (tested), each in
        //   the aforementioned range (tested), so the bytes can be extracted
        //   by casting each char to u8.

        let mut arr = [0u8; 20];
        let mut char_iter = value.chars();

        for a in arr.iter_mut() {
            if let Some(c) = char_iter.next() {
                if c as u32 > 255 {
                    return Err(E::custom(format!(
                        "character not in single byte range: {:#?}",
                        c
                    )));
                }

                *a = c as u8;
            } else {
                return Err(E::custom(format!("not 20 bytes: {:#?}", value)));
            }
        }

        Ok(arr)
    }
}

#[inline]
fn deserialize_20_bytes<'de, D>(deserializer: D) -> Result<[u8; 20], D::Error>
where
    D: Deserializer<'de>,
{
    deserializer.deserialize_any(TwentyByteVisitor)
}

#[cfg(test)]
mod tests {
    use quickcheck_macros::quickcheck;

    use crate::common::InfoHash;

    fn info_hash_from_bytes(bytes: &[u8]) -> InfoHash {
        let mut arr = [0u8; 20];

        assert!(bytes.len() == 20);

        arr.copy_from_slice(bytes);

        InfoHash(arr)
    }

    #[test]
    fn test_deserialize_20_bytes() {
        unsafe {
            let mut input = r#""aaaabbbbccccddddeeee""#.to_string();

            let expected = info_hash_from_bytes(b"aaaabbbbccccddddeeee");
            let observed: InfoHash = ::simd_json::serde::from_str(&mut input).unwrap();

            assert_eq!(observed, expected);
        }

        unsafe {
            let mut input = r#""aaaabbbbccccddddeee""#.to_string();
            let res_info_hash: Result<InfoHash, _> = ::simd_json::serde::from_str(&mut input);

            assert!(res_info_hash.is_err());
        }

        unsafe {
            let mut input = r#""aaaabbbbccccddddeee𝕊""#.to_string();
            let res_info_hash: Result<InfoHash, _> = ::simd_json::serde::from_str(&mut input);

            assert!(res_info_hash.is_err());
        }
    }

    #[test]
    fn test_serde_20_bytes() {
        let info_hash = info_hash_from_bytes(b"aaaabbbbccccddddeeee");

        let info_hash_2 = unsafe {
            let mut out = ::simd_json::serde::to_string(&info_hash).unwrap();

            ::simd_json::serde::from_str(&mut out).unwrap()
        };

        assert_eq!(info_hash, info_hash_2);
    }

    #[quickcheck]
    fn quickcheck_serde_20_bytes(info_hash: InfoHash) -> bool {
        unsafe {
            let mut out = ::simd_json::serde::to_string(&info_hash).unwrap();
            let info_hash_2 = ::simd_json::serde::from_str(&mut out).unwrap();

            info_hash == info_hash_2
        }
    }
}
