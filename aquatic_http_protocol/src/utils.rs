use std::net::{Ipv4Addr, Ipv6Addr};

use serde::Serializer;
use smartstring::{SmartString, LazyCompact};

use super::response::ResponsePeer;


pub fn urlencode_20_bytes(input: [u8; 20]) -> Vec<u8> {
    let mut tmp = [0u8; 40];

    hex::encode_to_slice(&input, &mut tmp);

    let mut output = Vec::with_capacity(60);

    for chunk in tmp.chunks_exact(2){
        output.push(b'%');
        output.extend_from_slice(chunk);
    }

    output
}


/// Not for serde
pub fn deserialize_20_bytes(value: SmartString<LazyCompact>) -> anyhow::Result<[u8; 20]> {
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


pub fn serialize_response_peers_ipv4<S>(
    response_peers: &[ResponsePeer<Ipv4Addr>],
    serializer: S
) -> Result<S::Ok, S::Error> where S: Serializer {
    let mut bytes = Vec::with_capacity(response_peers.len() * 6);

    for peer in response_peers {
        bytes.extend_from_slice(&u32::from(peer.ip_address).to_be_bytes());
        bytes.extend_from_slice(&peer.port.to_be_bytes())
    }

    serializer.serialize_bytes(&bytes)
}


pub fn serialize_response_peers_ipv6<S>(
    response_peers: &[ResponsePeer<Ipv6Addr>],
    serializer: S
) -> Result<S::Ok, S::Error> where S: Serializer {
    let mut bytes = Vec::with_capacity(response_peers.len() * 6);

    for peer in response_peers {
        bytes.extend_from_slice(&u128::from(peer.ip_address).to_be_bytes());
        bytes.extend_from_slice(&peer.port.to_be_bytes())
    }

    serializer.serialize_bytes(&bytes)
}