use std::net::{Ipv4Addr, Ipv6Addr};
use std::io::Write;

use anyhow::Context;
use serde::Serializer;
use smartstring::{SmartString, LazyCompact};

use super::response::ResponsePeer;


pub fn urlencode_20_bytes(
    input: [u8; 20],
    output: &mut impl Write
) -> ::std::io::Result<()> {
    let mut tmp = [0u8; 2];

    for i in 0..input.len() {
        hex::encode_to_slice(&input[i..i+1], &mut tmp).unwrap();

        output.write(b"%")?;
        output.write(&tmp)?;
    }

    Ok(())
}


pub fn urldecode_20_bytes(value: &str) -> anyhow::Result<[u8; 20]> {
    let mut out_arr = [0u8; 20];

    let mut chars = value.chars();

    for i in 0..20 {
        let c = chars.next()
            .with_context(|| "less than 20 chars")?;

        if c as u32 > 255 {
            return Err(anyhow::anyhow!(
                "character not in single byte range: {:#?}",
                c
            ));
        }

        if c == '%' {
            let first = chars.next()
                .with_context(|| "missing first urldecode char in pair")?;
            let second = chars.next()
                .with_context(|| "missing second urldecode char in pair")?;

            let hex = [first as u8, second as u8];

            hex::decode_to_slice(&hex, &mut out_arr[i..i+1]).map_err(|err|
                anyhow::anyhow!("hex decode error: {:?}", err)
            )?;
        } else {
            if c as u32 > 255 {
                return Err(anyhow::anyhow!(
                    "character not in single byte range: {:#?}",
                    c
                ));
            }

            out_arr[i] = c as u8;
        }
    }

    if chars.next().is_some(){
        return Err(anyhow::anyhow!("more than 20 chars"));
    }

    Ok(out_arr)
}


pub fn urldecode(value: &str) -> anyhow::Result<SmartString<LazyCompact>> {
    let mut processed = SmartString::new();

    let bytes = value.as_bytes();
    let iter = ::memchr::memchr_iter(b'%', bytes);

    let mut str_index_after_hex = 0usize;

    for i in iter {
        match (bytes.get(i), bytes.get(i + 1), bytes.get(i + 2)){
            (Some(0..=127), Some(0..=127), Some(0..=127)) => {
                if i > 0 {
                    processed.push_str(&value[str_index_after_hex..i]);
                }

                str_index_after_hex = i + 3;

                let hex = &value[i + 1..i + 3];
                let byte = u8::from_str_radix(&hex, 16)?;

                processed.push(byte as char);
            },
            _ => {
                return Err(anyhow::anyhow!(
                    "invalid urlencoded segment at byte {} in {}", i, value
                ));
            }
        }
    }

    if let Some(rest_of_str) = value.get(str_index_after_hex..){
        processed.push_str(rest_of_str);
    }

    Ok(processed)
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


#[cfg(test)]
mod tests {
    use quickcheck_macros::*;

    use super::*;

    #[test]
    fn test_urlencode_20_bytes(){
        let mut input = [0u8; 20];

        for (i, b) in input.iter_mut().enumerate(){
            *b = i as u8 % 10;
        }

        let mut output = Vec::new();

        urlencode_20_bytes(input, &mut output).unwrap();

        assert_eq!(output.len(), 60);

        for (i, chunk) in output.chunks_exact(3).enumerate(){
            // Not perfect but should do the job
            let reference = [b'%', b'0', input[i] + 48];

            let success = chunk == reference;

            if !success {
                println!("failing index: {}", i);
            }

            assert_eq!(chunk, reference);
        }
    }

    #[quickcheck]
    fn test_urlencode_urldecode_20_bytes(
        a: u8,
        b: u8,
        c: u8,
        d: u8,
        e: u8,
        f: u8,
        g: u8,
        h: u8,
    ) -> bool {
        let input: [u8; 20] = [
            a, b, c, d, e, f, g, h, b, c, d, a, e, f, g, h, a, b, d, c
        ];

        let mut output = Vec::new();

        urlencode_20_bytes(input, &mut output).unwrap();

        let s = ::std::str::from_utf8(&output).unwrap();

        let decoded = urldecode_20_bytes(s).unwrap();

        assert_eq!(input, decoded);

        input == decoded
    }

    #[test]
    fn test_urldecode(){
        assert_eq!(urldecode("").unwrap(), "".to_string());
        assert_eq!(urldecode("abc").unwrap(), "abc".to_string());
        assert_eq!(urldecode("%21").unwrap(), "!".to_string());
        assert_eq!(urldecode("%21%3D").unwrap(), "!=".to_string());
        assert_eq!(urldecode("abc%21def%3Dghi").unwrap(), "abc!def=ghi".to_string());
        assert!(urldecode("%").is_err());
        assert!(urldecode("%å7").is_err());
    }
}