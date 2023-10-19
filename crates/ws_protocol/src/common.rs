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

pub struct AnnounceActionVisitor;

impl<'de> Visitor<'de> for AnnounceActionVisitor {
    type Value = AnnounceAction;

    fn expecting(&self, formatter: &mut std::fmt::Formatter) -> std::fmt::Result {
        formatter.write_str("string with value 'announce'")
    }

    fn visit_str<E>(self, v: &str) -> Result<Self::Value, E>
    where
        E: ::serde::de::Error,
    {
        if v == "announce" {
            Ok(AnnounceAction)
        } else {
            Err(E::custom("value is not 'announce'"))
        }
    }
}

pub struct ScrapeActionVisitor;

impl<'de> Visitor<'de> for ScrapeActionVisitor {
    type Value = ScrapeAction;

    fn expecting(&self, formatter: &mut std::fmt::Formatter) -> std::fmt::Result {
        formatter.write_str("string with value 'scrape'")
    }

    fn visit_str<E>(self, v: &str) -> Result<Self::Value, E>
    where
        E: ::serde::de::Error,
    {
        if v == "scrape" {
            Ok(ScrapeAction)
        } else {
            Err(E::custom("value is not 'scrape'"))
        }
    }
}

fn serialize_20_bytes<S>(data: &[u8; 20], serializer: S) -> Result<S::Ok, S::Error>
where
    S: Serializer,
{
    let text: String = data.iter().map(|byte| char::from(*byte)).collect();

    serializer.serialize_str(&text)
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

        arr.copy_from_slice(&bytes[..]);

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
