use anyhow::Context;
use hashbrown::HashMap;
use smartstring::{SmartString, LazyCompact};

use super::common::*;
use super::utils::*;


#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AnnounceRequest {
    pub info_hash: InfoHash,
    pub peer_id: PeerId,
    pub port: u16,
    pub bytes_left: usize,
    pub event: AnnounceEvent,
    pub compact: bool,
    /// Number of response peers wanted
    pub numwant: Option<usize>,
    pub key: Option<SmartString<LazyCompact>>,
}


impl AnnounceRequest {
    pub fn as_bytes(&self) -> Vec<u8> {
        let mut bytes = Vec::with_capacity(
            24 +
            60 +
            9 +
            60 +
            6 +
            5 + // high estimate
            6 +
            2 + // estimate
            14 + // FIXME event
            9 + 
            1 +
            20 + // numwant bad estimate
            20 + // key bad estimate
            13
        );

        bytes.extend_from_slice(b"GET /announce?info_hash=");
        urlencode_20_bytes(self.info_hash.0, &mut bytes);

        bytes.extend_from_slice(b"&peer_id=");
        urlencode_20_bytes(self.info_hash.0, &mut bytes);

        bytes.extend_from_slice(b"&port=");
        let _ = itoa::write(&mut bytes, self.port);

        bytes.extend_from_slice(b"&left=");
        let _ = itoa::write(&mut bytes, self.bytes_left);

        bytes.extend_from_slice(b"&event=started"); // FIXME

        bytes.extend_from_slice(b"&compact=");
        let _ = itoa::write(&mut bytes, self.compact as u8);

        if let Some(numwant) = self.numwant {
            bytes.extend_from_slice(b"&numwant=");
            let _ = itoa::write(&mut bytes, numwant);
        }

        if let Some(ref key) = self.key {
            bytes.extend_from_slice(b"&key=");
            bytes.extend_from_slice(key.as_str().as_bytes());
        }

        bytes.extend_from_slice(b" HTTP/1.1\r\n\r\n");

        bytes
    }
}


#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ScrapeRequest {
    pub info_hashes: Vec<InfoHash>,
}


impl ScrapeRequest {
    pub fn as_bytes(&self) -> Vec<u8> {
        let mut bytes = Vec::new();

        bytes.extend_from_slice(b"GET /scrape?");

        let mut first = true;

        for info_hash in self.info_hashes.iter() {
            if !first {
                bytes.push(b'&')
            }

            bytes.extend_from_slice(b"info_hash=");

            for b in info_hash.0.iter() {
                bytes.push(b'%');
                bytes.extend_from_slice(format!("{:02x}", b).as_bytes());
            }

            first = false;
        }

        bytes.extend_from_slice(b" HTTP/1.1\r\n\r\n");

        bytes
    }
}


#[derive(Debug)]
pub enum RequestParseError {
    NeedMoreData,
    Invalid(anyhow::Error),
}


#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Request {
    Announce(AnnounceRequest),
    Scrape(ScrapeRequest),
}


impl Request {
    /// Parse Request from HTTP request bytes
    pub fn from_bytes(bytes: &[u8]) -> Result<Self, RequestParseError> {
        let mut headers = [httparse::EMPTY_HEADER; 16];
        let mut http_request = httparse::Request::new(&mut headers);

        match http_request.parse(bytes){
            Ok(httparse::Status::Complete(_)) => {
                if let Some(path) = http_request.path {
                    let res_request = Self::from_http_get_path(path);

                    res_request.map_err(RequestParseError::Invalid)
                } else {
                    Err(RequestParseError::Invalid(anyhow::anyhow!("no http path")))
                }
            },
            Ok(httparse::Status::Partial) => {
                Err(RequestParseError::NeedMoreData)
            },
            Err(err) => {
                Err(RequestParseError::Invalid(anyhow::anyhow!("httparse: {:?}", err)))
            }
        }
    }

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
 
        Self::parse_key_value_pairs_memchr(
            &mut info_hashes,
            &mut data,
            query_string
        )?;

        if location == "/announce" {
            let numwant = if let Some(s) = data.remove("numwant"){
                let numwant = s.parse::<usize>()
                    .map_err(|err|
                        anyhow::anyhow!("parse 'numwant': {}", err)
                    )?;
                
                Some(numwant)
            } else {
                None
            };
            let key = if let Some(s) = data.remove("key"){
                if s.len() > 100 {
                    return Err(anyhow::anyhow!("'key' is too long"))
                }

                Some(s)
            } else {
                None
            };
            let port = if let Some(port) = data.remove("port"){
                port.parse().with_context(|| "parse port")?
            } else {
                return Err(anyhow::anyhow!("no port"));
            };
            let bytes_left = if let Some(left) = data.remove("left"){
                left.parse().with_context(|| "parse bytes left")?
            } else {
                return Err(anyhow::anyhow!("no left"));
            };
            let event = if let Some(event) = data.remove("event"){
                if let Ok(event) = event.parse(){
                    event
                } else {
                    return Err(anyhow::anyhow!("invalid event: {}", event));
                }
            } else {
                AnnounceEvent::default()
            };
            let compact = if let Some(compact) = data.remove("compact"){
                if compact.as_str() == "1" {
                    true
                } else {
                    return Err(anyhow::anyhow!("compact set, but not to 1"));
                }
            } else {
                true
            };

            let request = AnnounceRequest {
                info_hash: info_hashes.pop()
                    .with_context(|| "no info_hash")
                    .and_then(deserialize_20_bytes)
                    .map(InfoHash)?,
                peer_id: data.remove("peer_id")
                    .with_context(|| "no peer_id")
                    .and_then(deserialize_20_bytes)
                    .map(PeerId)?,
                port,
                bytes_left,
                event,
                compact,
                numwant,
                key,
            };

            Ok(Request::Announce(request))
        } else {
            let mut parsed_info_hashes = Vec::with_capacity(info_hashes.len());

            for info_hash in info_hashes {
                parsed_info_hashes.push(InfoHash(deserialize_20_bytes(info_hash)?));
            }

            let request = ScrapeRequest {
                info_hashes: parsed_info_hashes,
            };

            Ok(Request::Scrape(request))
        }
    }

    /// Seems to be somewhat faster than non-memchr version
    fn parse_key_value_pairs_memchr<'a>(
        info_hashes: &mut Vec<SmartString<LazyCompact>>,
        data: &mut HashMap<&'a str, SmartString<LazyCompact>>,
        query_string: &'a str,
    ) -> anyhow::Result<()> {
        let query_string_bytes = query_string.as_bytes();

        let mut ampersand_iter = ::memchr::memchr_iter(b'&', query_string_bytes);
        let mut position = 0usize;

        for equal_sign_index in ::memchr::memchr_iter(b'=', query_string_bytes){
            let segment_end = ampersand_iter.next()
                .unwrap_or(query_string.len());

            let key = query_string.get(position..equal_sign_index)
                .with_context(|| format!("no key at {}..{}", position, equal_sign_index))?;
            let value = query_string.get(equal_sign_index + 1..segment_end)
                .with_context(|| format!("no value at {}..{}", equal_sign_index + 1, segment_end))?;
            
            // whitelist keys to avoid having to use ddos-resistant hashmap
            match key {
                "info_hash" => {
                    let value = Self::urldecode_memchr(value)?;

                    info_hashes.push(value);
                },
                "peer_id" | "port" | "left" | "event" | "compact" | "numwant" | "key" => {
                    let value = Self::urldecode_memchr(value)?;

                    data.insert(key, value);
                },
                k => {
                    ::log::info!("ignored unrecognized key: {}", k)
                }
            }

            if segment_end == query_string.len(){
                break
            } else {
                position = segment_end + 1;
            }
        }

        Ok(())
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

    /// Quite a bit faster than non-memchr version
    fn urldecode_memchr(value: &str) -> anyhow::Result<SmartString<LazyCompact>> {
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

    pub fn as_bytes(&self) -> Vec<u8> {
        match self {
            Self::Announce(r) => r.as_bytes(),
            Self::Scrape(r) => r.as_bytes(),
        }
    }
}


#[cfg(test)]
mod tests {
    use super::*;

    static ANNOUNCE_REQUEST_PATH: &str = "/announce?info_hash=%04%0bkV%3f%5cr%14%a6%b7%98%adC%c3%c9.%40%24%00%b9&peer_id=-ABC940-5ert69muw5t8&port=12345&uploaded=0&downloaded=0&left=1&numwant=0&key=4ab4b877&compact=1&supportcrypto=1&event=started";
    static SCRAPE_REQUEST_PATH: &str = "/scrape?info_hash=%04%0bkV%3f%5cr%14%a6%b7%98%adC%c3%c9.%40%24%00%b9";
    static REFERENCE_INFO_HASH: [u8; 20] = [0x04, 0x0b, b'k', b'V', 0x3f, 0x5c, b'r', 0x14, 0xa6, 0xb7, 0x98, 0xad, b'C', 0xc3, 0xc9, b'.', 0x40, 0x24, 0x00, 0xb9];
    static REFERENCE_PEER_ID: [u8; 20] = [b'-', b'A', b'B', b'C', b'9', b'4', b'0', b'-', b'5', b'e', b'r', b't', b'6', b'9', b'm', b'u', b'w', b'5', b't', b'8'];

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

    fn get_reference_announce_request() -> Request {
        Request::Announce(AnnounceRequest {
            info_hash: InfoHash(REFERENCE_INFO_HASH),
            peer_id: PeerId(REFERENCE_PEER_ID),
            port: 12345,
            bytes_left: 1,
            event: AnnounceEvent::Started,
            compact: true,
            numwant: Some(0),
            key: Some("4ab4b877".into())
        })
    }

    #[test]
    fn test_announce_request_from_bytes(){
        let mut bytes = Vec::new();

        bytes.extend_from_slice(b"GET ");
        bytes.extend_from_slice(&ANNOUNCE_REQUEST_PATH.as_bytes());
        bytes.extend_from_slice(b" HTTP/1.1\r\n\r\n");

        let parsed_request = Request::from_bytes(
            &bytes[..]
        ).unwrap();

        let reference_request = get_reference_announce_request();

        assert_eq!(parsed_request, reference_request);
    }

    #[test]
    fn test_announce_request_from_path(){
        let parsed_request = Request::from_http_get_path(
            ANNOUNCE_REQUEST_PATH
        ).unwrap();

        let reference_request = get_reference_announce_request();

        assert_eq!(parsed_request, reference_request);
    }

    #[test]
    fn test_scrape_request_from_path(){
        let parsed_request = Request::from_http_get_path(
            SCRAPE_REQUEST_PATH
        ).unwrap();

        let reference_request = Request::Scrape(ScrapeRequest {
            info_hashes: vec![InfoHash(REFERENCE_INFO_HASH)],
        });

        assert_eq!(parsed_request, reference_request);
    }
}