use std::net::IpAddr;

use serde::{Serializer, Deserializer, de::{Visitor, SeqAccess}};

use super::{InfoHash, ResponsePeer};


struct BoolFromNumberVisitor;

impl<'de> Visitor<'de> for BoolFromNumberVisitor {
    type Value = bool;

    fn expecting(&self, formatter: &mut std::fmt::Formatter) -> std::fmt::Result {
        formatter.write_str("1 for true, 0 for false")
    }

    #[inline]
    fn visit_str<E>(self, value: &str) -> Result<Self::Value, E>
        where E: ::serde::de::Error,
    {
        if value == "0" {
            Ok(false)
        } else if value == "1" {
            Ok(true)
        } else {
            Err(E::custom(format!("not 0 or 1: {}", value)))
        }
    }
}


#[inline]
pub fn deserialize_bool_from_number<'de, D>(
    deserializer: D
) -> Result<bool, D::Error>
    where D: Deserializer<'de>
{
    deserializer.deserialize_any(BoolFromNumberVisitor)
}



/// Decode string of 20 byte-size chars to a [u8; 20]
struct TwentyByteVisitor;

impl<'de> Visitor<'de> for TwentyByteVisitor {
    type Value = [u8; 20];

    fn expecting(&self, formatter: &mut std::fmt::Formatter) -> std::fmt::Result {
        formatter.write_str("string consisting of 20 bytes")
    }

    #[inline]
    fn visit_str<E>(self, value: &str) -> Result<Self::Value, E>
        where E: ::serde::de::Error,
    {
        let mut arr = [0u8; 20];
        let mut char_iter = value.chars();

        for a in arr.iter_mut(){
            if let Some(c) = char_iter.next(){
                if c as u32 > 255 {
                    return Err(E::custom(format!(
                        "character not in single byte range: {:#?}",
                        c
                    )));
                }

                *a = c as u8;
            } else {
                return Err(E::custom(format!("less than 20 bytes: {:#?}", value)));
            }
        }

        if char_iter.next().is_some(){
            Err(E::custom(format!("more than 20 bytes: {:#?}", value)))
        } else {
            Ok(arr)
        }
    }
}


#[inline]
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

    #[inline]
    fn visit_str<E>(self, value: &str) -> Result<Self::Value, E>
        where E: ::serde::de::Error,
    {
        match TwentyByteVisitor::visit_str::<E>(TwentyByteVisitor, value){
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
            let arr = TwentyByteVisitor::visit_str(
                TwentyByteVisitor, value
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