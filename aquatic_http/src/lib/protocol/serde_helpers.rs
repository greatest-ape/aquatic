use std::net::IpAddr;

use serde::{Serializer, Deserializer, de::{Visitor, SeqAccess}};

use super::{InfoHash, ResponsePeer};


struct TwentyCharStringVisitor;

impl<'de> Visitor<'de> for TwentyCharStringVisitor {
    type Value = String;

    fn expecting(&self, formatter: &mut std::fmt::Formatter) -> std::fmt::Result {
        formatter.write_str("string consisting of 20 chars")
    }

    #[inline]
    fn visit_str<E>(self, value: &str) -> Result<Self::Value, E>
        where E: ::serde::de::Error,
    {
        if value.chars().count() == 20 {
            Ok(value.to_string())
        } else {
            Err(E::custom(format!("not 20 chars: {:#?}", value)))
        }
    }
}


#[inline]
pub fn deserialize_20_char_string<'de, D>(
    deserializer: D
) -> Result<String, D::Error>
    where D: Deserializer<'de>
{
    deserializer.deserialize_any(TwentyCharStringVisitor)
}


pub struct InfoHashVecVisitor;


impl<'de> Visitor<'de> for InfoHashVecVisitor {
    type Value = Vec<InfoHash>;

    fn expecting(&self, formatter: &mut std::fmt::Formatter) -> std::fmt::Result {
        formatter.write_str("string or array of strings consisting of 20 bytes")
    }

    #[inline]
    fn visit_str<E>(self, value: &str) -> Result<Self::Value, E>
        where E: ::serde::de::Error,
    {
        match TwentyCharStringVisitor::visit_str::<E>(TwentyCharStringVisitor, value){
            Ok(arr) => Ok(vec![InfoHash(arr)]),
            Err(err) => Err(E::custom(format!("got string, but {}", err)))
        }
    }

    #[inline]
    fn visit_seq<A>(self, mut seq: A) -> Result<Self::Value, A::Error>
        where A: SeqAccess<'de>
    {
        let mut info_hashes: Self::Value = Vec::new();

        while let Ok(Some(value)) = seq.next_element::<&str>(){
            let arr = TwentyCharStringVisitor::visit_str(
                TwentyCharStringVisitor, value
            )?;

            info_hashes.push(InfoHash(arr));
        }

        Ok(info_hashes)
    }

    #[inline]
    fn visit_none<E>(self) -> Result<Self::Value, E>
        where E: ::serde::de::Error
    {
        Ok(vec![])
    }
}


/// Empty vector is returned if value is null or any invalid info hash
/// is present
#[inline]
pub fn deserialize_info_hashes<'de, D>(
    deserializer: D
) -> Result<Vec<InfoHash>, D::Error>
    where D: Deserializer<'de>,
{
    Ok(deserializer.deserialize_any(InfoHashVecVisitor).unwrap_or_default())
}


pub fn serialize_response_peers_compact<S>(
    response_peers: &Vec<ResponsePeer>,
    serializer: S
) -> Result<S::Ok, S::Error> where S: Serializer {
    let mut bytes = Vec::with_capacity(response_peers.len() * 6);

    for peer in response_peers {
        match peer.ip_address {
            IpAddr::V4(ip) => {
                bytes.extend_from_slice(&u32::from(ip).to_be_bytes());
                bytes.extend_from_slice(&peer.port.to_be_bytes())
            },
            IpAddr::V6(_) => {
                continue
            }
        }
    }

    let text: String = bytes.into_iter().map(|byte| byte as char).collect();

    serializer.serialize_str(&text)
}