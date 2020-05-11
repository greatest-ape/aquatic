use serde::{Deserializer, de::{Visitor, SeqAccess}};

use super::InfoHash;


struct TwentyByteVisitor;

impl<'de> Visitor<'de> for TwentyByteVisitor {
    type Value = [u8; 20];

    fn expecting(&self, formatter: &mut std::fmt::Formatter) -> std::fmt::Result {
        formatter.write_str("string consisting of 20 bytes")
    }

    fn visit_str<E>(self, value: &str) -> Result<Self::Value, E>
        where E: ::serde::de::Error,
    {
        // Value is encoded in nodejs reference client something as follows:
        // ```
        // var infoHash = 'abcd..'; // 40 hexadecimals
        // Buffer.from(infoHash, 'hex').toString('binary');
        // ```
        // which produces a UTF16 string with each char having only the low
        // byte set. Here, we extract it by casting to u8.

        let mut arr = [0u8; 20];
        let mut max_i = 0;

        for (i, (a, c)) in arr.iter_mut().zip(value.chars()).enumerate(){
            if c as u32 > 255 {
                return Err(E::custom(format!(
                    "character not in single byte range: {}",
                    c
                )))
            }
            *a = c as u8;
            max_i = i;
        }

        if max_i == 19 {
            Ok(arr)
        } else {
            Err(E::custom(format!("not 20 bytes: {}", value)))
        }
    }
}


pub fn deserialize_20_bytes<'de, D>(
    deserializer: D
) -> Result<[u8; 20], D::Error>
    where D: Deserializer<'de>
{
    deserializer.deserialize_any(TwentyByteVisitor)
}


pub struct InfoHashVecVisitor;


impl<'de> Visitor<'de> for InfoHashVecVisitor {
    type Value = Vec<InfoHash>;

    fn expecting(&self, formatter: &mut std::fmt::Formatter) -> std::fmt::Result {
        formatter.write_str("string or array of strings consisting of 20 bytes")
    }

    fn visit_str<E>(self, value: &str) -> Result<Self::Value, E>
        where E: ::serde::de::Error,
    {
        match TwentyByteVisitor::visit_str::<E>(TwentyByteVisitor, value){
            Ok(arr) => Ok(vec![InfoHash(arr)]),
            Err(err) => Err(E::custom(format!("got string, but {}", err)))
        }
    }

    fn visit_seq<A>(self, mut seq: A) -> Result<Self::Value, A::Error>
        where A: SeqAccess<'de>
    {
        let mut info_hashes: Self::Value = Vec::new();

        while let Ok(Some(value)) = seq.next_element::<&str>(){
            let arr = TwentyByteVisitor::visit_str(
                TwentyByteVisitor, value
            )?;

            info_hashes.push(InfoHash(arr));
        }

        Ok(info_hashes)
    }

    fn visit_none<E>(self) -> Result<Self::Value, E>
        where E: ::serde::de::Error
    {
        Ok(vec![])
    }
}


/// Empty vector is returned if value is null or any invalid info hash
/// is present
pub fn deserialize_info_hashes<'de, D>(
    deserializer: D
) -> Result<Vec<InfoHash>, D::Error>
    where D: Deserializer<'de>,
{
    Ok(deserializer.deserialize_any(InfoHashVecVisitor).unwrap_or_default())
}


#[cfg(test)]
mod tests {
    use serde::Deserialize;

    use super::*;

    fn info_hash_from_bytes(bytes: &[u8]) -> InfoHash {
        let mut arr = [0u8; 20];

        assert!(bytes.len() == 20);

        arr.copy_from_slice(&bytes[..]);

        InfoHash(arr)
    }

    #[test]
    fn test_deserialize_20_bytes(){
        let input = r#""aaaabbbbccccddddeeee""#;

        let expected = info_hash_from_bytes(b"aaaabbbbccccddddeeee");
        let observed: InfoHash = serde_json::from_str(input).unwrap();

        assert_eq!(observed, expected);

        let input = r#""aaaabbbbccccddddeee""#;
        let res_info_hash: Result<InfoHash, _> = serde_json::from_str(input);

        assert!(res_info_hash.is_err());

        let input = r#""aaaabbbbccccddddeee𝕊""#;
        let res_info_hash: Result<InfoHash, _> = serde_json::from_str(input);

        assert!(res_info_hash.is_err());
    }

    #[derive(Debug, PartialEq, Eq, Deserialize)]
    struct Test {
        #[serde(deserialize_with = "deserialize_info_hashes", default)]
        info_hashes: Vec<InfoHash>,
    }


    #[test]
    fn test_deserialize_info_hashes_vec(){
        let input = r#"{
            "info_hashes": ["aaaabbbbccccddddeeee", "aaaabbbbccccddddeeee"]
        }"#;

        let expected = Test {
            info_hashes: vec![
                info_hash_from_bytes(b"aaaabbbbccccddddeeee"),
                info_hash_from_bytes(b"aaaabbbbccccddddeeee"),
            ]
        };

        let observed: Test = serde_json::from_str(input).unwrap();

        assert_eq!(observed, expected);
    }

    #[test]
    fn test_deserialize_info_hashes_str(){
        let input = r#"{
            "info_hashes": "aaaabbbbccccddddeeee"
        }"#;

        let expected = Test {
            info_hashes: vec![
                info_hash_from_bytes(b"aaaabbbbccccddddeeee"),
            ]
        };

        let observed: Test = serde_json::from_str(input).unwrap();

        assert_eq!(observed, expected);
    }

    #[test]
    fn test_deserialize_info_hashes_null(){
        let input = r#"{
            "info_hashes": null
        }"#;

        let expected = Test {
            info_hashes: vec![]
        };

        let observed: Test = serde_json::from_str(input).unwrap();

        assert_eq!(observed, expected);
    }

    #[test]
    fn test_deserialize_info_hashes_missing(){
        let input = r#"{}"#;

        let expected = Test {
            info_hashes: vec![]
        };

        let observed: Test = serde_json::from_str(input).unwrap();

        assert_eq!(observed, expected);
    }
}