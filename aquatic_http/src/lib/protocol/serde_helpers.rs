use std::net::IpAddr;

use serde::Serializer;

use super::ResponsePeer;



/// Not for serde
pub fn deserialize_20_bytes(value: &str) -> anyhow::Result<[u8; 20]> {
    let mut arr = [0u8; 20];
    let mut char_iter = value.chars();

    for a in arr.iter_mut(){
        if let Some(c) = char_iter.next(){
            if c as u32 > 255 {
                return Err(anyhow::anyhow!(
                    "character not in single byte range: {:#?}",
                    c
                ));
            }

            *a = c as u8;
        } else {
            return Err(anyhow::anyhow!("less than 20 bytes: {:#?}", value));
        }
    }

    if char_iter.next().is_some(){
        Err(anyhow::anyhow!("more than 20 bytes: {:#?}", value))
    } else {
        Ok(arr)
    }
}


#[inline]
pub fn serialize_20_bytes<S>(
    bytes: &[u8; 20],
    serializer: S
) -> Result<S::Ok, S::Error> where S: Serializer {
    serializer.serialize_bytes(bytes)
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
                continue;
            }
        }
    }

    serializer.serialize_bytes(&bytes)
}