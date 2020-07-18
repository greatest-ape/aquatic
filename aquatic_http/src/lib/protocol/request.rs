use anyhow::Context;
use hashbrown::HashMap;

use super::common::*;
use super::utils::*;


#[derive(Debug, Clone)]
pub struct AnnounceRequest {
    pub info_hash: InfoHash,
    pub peer_id: PeerId,
    pub port: u16,
    pub bytes_left: usize,
    pub event: AnnounceEvent,
    pub compact: bool,
    /// Number of response peers wanted
    pub numwant: Option<usize>,
    pub key: Option<String>,
}


#[derive(Debug, Clone)]
pub struct ScrapeRequest {
    pub info_hashes: Vec<InfoHash>,
}


#[derive(Debug, Clone)]
pub enum Request {
    Announce(AnnounceRequest),
    Scrape(ScrapeRequest),
}


impl Request {
    /// Parse Request from http path (GET `/announce?info_hash=...`)
    ///
    /// Existing serde-url decode crates were insufficient, so the decision was
    /// made to create a custom parser. serde_urlencoded doesn't support multiple
    /// values with same key, and serde_qs pulls in lots of dependencies. Both
    /// would need preprocessing for the binary format used for info_hash and
    /// peer_id.
    pub fn from_http_get_path(path: &str) -> anyhow::Result<Self> {
        ::log::debug!("request GET path: {}", path);

        let mut split_parts= path.splitn(2, '?');

        let location = split_parts.next()
            .with_context(|| "no location")?;
        let query_string = split_parts.next()
            .with_context(|| "no query string")?;

        let mut info_hashes = Vec::new();
        let mut data = HashMap::new();

        for part in query_string.split('&'){
            let mut key_and_value = part.splitn(2, '=');

            let key = key_and_value.next()
                .with_context(|| format!("no key in {}", part))?;
            let value = key_and_value.next()
                .with_context(|| format!("no value in {}", part))?;
            let value = Self::urldecode_memchr(value)?;

            if key == "info_hash" {
                info_hashes.push(value);
            } else {
                data.insert(key, value);
            }
        }

        if location == "/announce" {
            let numwant = if let Some(s) = data.get("numwant"){
                let numwant = s.parse::<usize>()
                    .map_err(|err|
                        anyhow::anyhow!("parse 'numwant': {}", err)
                    )?;
                
                Some(numwant)
            } else {
                None
            };
            let key = if let Some(s) = data.get("key"){
                if s.len() > 100 {
                    return Err(anyhow::anyhow!("'key' is too long"))
                }

                Some(s.clone())
            } else {
                None
            };

            let request = AnnounceRequest {
                info_hash: info_hashes.get(0)
                    .with_context(|| "no info_hash")
                    .and_then(|s| deserialize_20_bytes(s))
                    .map(InfoHash)?,
                peer_id: data.get("peer_id")
                    .with_context(|| "no peer_id")
                    .and_then(|s| deserialize_20_bytes(s))
                    .map(PeerId)?,
                port: data.get("port")
                    .with_context(|| "no port")
                    .and_then(|s| s.parse()
                    .map_err(|err| anyhow::anyhow!("parse 'port': {}", err)))?,
                bytes_left: data.get("left")
                    .with_context(|| "no left")
                    .and_then(|s| s.parse()
                    .map_err(|err| anyhow::anyhow!("parse 'left': {}", err)))?,
                event: data.get("event")
                    .and_then(|s| s.parse().ok())
                    .unwrap_or_default(),
                compact: data.get("compact")
                    .map(|s| s == "1")
                    .unwrap_or(true),
                numwant,
                key,
            };

            Ok(Request::Announce(request))
        } else {
            let mut parsed_info_hashes = Vec::with_capacity(info_hashes.len());

            for info_hash in info_hashes {
                parsed_info_hashes.push(InfoHash(deserialize_20_bytes(&info_hash)?));
            }

            let request = ScrapeRequest {
                info_hashes: parsed_info_hashes,
            };

            Ok(Request::Scrape(request))
        }
    }

    /// The info hashes and peer id's that are received are url-encoded byte
    /// by byte, e.g., %fa for byte 0xfa. However, they need to be parsed as
    /// UTF-8 string, meaning that non-ascii bytes are invalid characters.
    /// Therefore, these bytes must be converted to their equivalent multi-byte
    /// UTF-8 encodings.
    fn urldecode(value: &str) -> anyhow::Result<String> {
        let mut processed = String::new();

        for (i, part) in value.split('%').enumerate(){
            if i == 0 {
                processed.push_str(part);
            } else if part.len() >= 2 {
                let mut two_first = String::with_capacity(2);

                for (j, c) in part.chars().enumerate(){
                    if j == 0 {
                        two_first.push(c);
                    } else if j == 1 {
                        two_first.push(c);

                        let byte = u8::from_str_radix(&two_first, 16)?;

                        processed.push(byte as char);
                    } else {
                        processed.push(c);
                    }
                }
            } else {
                return Err(anyhow::anyhow!(
                    "url decode: too few characters in '%{}'", part
                ))
            }
        }

        Ok(processed)
    }

    fn urldecode_memchr(value: &str) -> anyhow::Result<String> {
        let mut processed = String::with_capacity(value.len());

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

        processed.shrink_to_fit();

        Ok(processed)
    }
}


#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_urldecode(){
        let f = Request::urldecode_memchr;

        assert_eq!(f("").unwrap(), "".to_string());
        assert_eq!(f("abc").unwrap(), "abc".to_string());
        assert_eq!(f("%21").unwrap(), "!".to_string());
        assert_eq!(f("%21%3D").unwrap(), "!=".to_string());
        assert_eq!(f("abc%21def%3Dghi").unwrap(), "abc!def=ghi".to_string());
        assert!(f("%").is_err());
        assert!(f("%å7").is_err());
    }
}