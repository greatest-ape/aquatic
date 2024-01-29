use std::io::Write;

use anyhow::Context;
use compact_str::CompactString;

use super::common::*;
use super::utils::*;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AnnounceRequest {
    pub info_hash: InfoHash,
    pub peer_id: PeerId,
    pub port: u16,
    pub bytes_uploaded: usize,
    pub bytes_downloaded: usize,
    pub bytes_left: usize,
    pub event: AnnounceEvent,
    /// Number of response peers wanted
    pub numwant: Option<usize>,
    pub key: Option<CompactString>,
}

impl AnnounceRequest {
    fn write_bytes<W: Write>(&self, output: &mut W, url_suffix: &[u8]) -> ::std::io::Result<()> {
        output.write_all(b"GET /announce")?;
        output.write_all(url_suffix)?;
        output.write_all(b"?info_hash=")?;
        urlencode_20_bytes(self.info_hash.0, output)?;

        output.write_all(b"&peer_id=")?;
        urlencode_20_bytes(self.peer_id.0, output)?;

        output.write_all(b"&port=")?;
        output.write_all(itoa::Buffer::new().format(self.port).as_bytes())?;

        output.write_all(b"&uploaded=")?;
        output.write_all(itoa::Buffer::new().format(self.bytes_uploaded).as_bytes())?;

        output.write_all(b"&downloaded=")?;
        output.write_all(itoa::Buffer::new().format(self.bytes_downloaded).as_bytes())?;

        output.write_all(b"&left=")?;
        output.write_all(itoa::Buffer::new().format(self.bytes_left).as_bytes())?;

        match self.event {
            AnnounceEvent::Started => output.write_all(b"&event=started")?,
            AnnounceEvent::Stopped => output.write_all(b"&event=stopped")?,
            AnnounceEvent::Completed => output.write_all(b"&event=completed")?,
            AnnounceEvent::Empty => (),
        };

        if let Some(numwant) = self.numwant {
            output.write_all(b"&numwant=")?;
            output.write_all(itoa::Buffer::new().format(numwant).as_bytes())?;
        }

        if let Some(ref key) = self.key {
            output.write_all(b"&key=")?;
            output.write_all(::urlencoding::encode(key.as_str()).as_bytes())?;
        }

        // Always ask for compact responses to ease load testing of non-aquatic trackers
        output.write_all(b"&compact=1")?;

        output.write_all(b" HTTP/1.1\r\nHost: localhost\r\n\r\n")?;

        Ok(())
    }

    pub fn parse_query_string(query_string: &str) -> anyhow::Result<Self> {
        // -- Parse key-value pairs

        let mut opt_info_hash = None;
        let mut opt_peer_id = None;
        let mut opt_port = None;
        let mut opt_bytes_left = None;
        let mut opt_bytes_uploaded = None;
        let mut opt_bytes_downloaded = None;
        let mut event = AnnounceEvent::default();
        let mut opt_numwant = None;
        let mut opt_key = None;

        let query_string_bytes = query_string.as_bytes();

        let mut ampersand_iter = ::memchr::memchr_iter(b'&', query_string_bytes);
        let mut position = 0usize;

        for equal_sign_index in ::memchr::memchr_iter(b'=', query_string_bytes) {
            let segment_end = ampersand_iter.next().unwrap_or(query_string.len());

            let key = query_string
                .get(position..equal_sign_index)
                .with_context(|| format!("no key at {}..{}", position, equal_sign_index))?;
            let value = query_string
                .get(equal_sign_index + 1..segment_end)
                .with_context(|| {
                    format!("no value at {}..{}", equal_sign_index + 1, segment_end)
                })?;

            match key {
                "info_hash" => {
                    let value = urldecode_20_bytes(value)?;

                    opt_info_hash = Some(InfoHash(value));
                }
                "peer_id" => {
                    let value = urldecode_20_bytes(value)?;

                    opt_peer_id = Some(PeerId(value));
                }
                "port" => {
                    opt_port = Some(value.parse::<u16>().with_context(|| "parse port")?);
                }
                "left" => {
                    opt_bytes_left = Some(value.parse::<usize>().with_context(|| "parse left")?);
                }
                "uploaded" => {
                    opt_bytes_uploaded =
                        Some(value.parse::<usize>().with_context(|| "parse uploaded")?);
                }
                "downloaded" => {
                    opt_bytes_downloaded =
                        Some(value.parse::<usize>().with_context(|| "parse downloaded")?);
                }
                "event" => {
                    event = value
                        .parse::<AnnounceEvent>()
                        .map_err(|err| anyhow::anyhow!("invalid event: {}", err))?;
                }
                "compact" => {
                    if value != "1" {
                        return Err(anyhow::anyhow!("compact set, but not to 1"));
                    }
                }
                "numwant" => {
                    opt_numwant = Some(value.parse::<usize>().with_context(|| "parse numwant")?);
                }
                "key" => {
                    if value.len() > 100 {
                        return Err(anyhow::anyhow!("'key' is too long"));
                    }
                    opt_key = Some(::urlencoding::decode(value)?.into());
                }
                k => {
                    ::log::debug!("ignored unrecognized key: {}", k)
                }
            }

            if segment_end == query_string.len() {
                break;
            } else {
                position = segment_end + 1;
            }
        }

        Ok(AnnounceRequest {
            info_hash: opt_info_hash.with_context(|| "no info_hash")?,
            peer_id: opt_peer_id.with_context(|| "no peer_id")?,
            port: opt_port.with_context(|| "no port")?,
            bytes_uploaded: opt_bytes_uploaded.with_context(|| "no uploaded")?,
            bytes_downloaded: opt_bytes_downloaded.with_context(|| "no downloaded")?,
            bytes_left: opt_bytes_left.with_context(|| "no left")?,
            event,
            numwant: opt_numwant,
            key: opt_key,
        })
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ScrapeRequest {
    pub info_hashes: Vec<InfoHash>,
}

impl ScrapeRequest {
    fn write_bytes<W: Write>(&self, output: &mut W, url_suffix: &[u8]) -> ::std::io::Result<()> {
        output.write_all(b"GET /scrape")?;
        output.write_all(url_suffix)?;
        output.write_all(b"?")?;

        let mut first = true;

        for info_hash in self.info_hashes.iter() {
            if !first {
                output.write_all(b"&")?;
            }

            output.write_all(b"info_hash=")?;
            urlencode_20_bytes(info_hash.0, output)?;

            first = false;
        }

        output.write_all(b" HTTP/1.1\r\nHost: localhost\r\n\r\n")?;

        Ok(())
    }

    pub fn parse_query_string(query_string: &str) -> anyhow::Result<Self> {
        // -- Parse key-value pairs

        let mut info_hashes = Vec::new();

        let query_string_bytes = query_string.as_bytes();

        let mut ampersand_iter = ::memchr::memchr_iter(b'&', query_string_bytes);
        let mut position = 0usize;

        for equal_sign_index in ::memchr::memchr_iter(b'=', query_string_bytes) {
            let segment_end = ampersand_iter.next().unwrap_or(query_string.len());

            let key = query_string
                .get(position..equal_sign_index)
                .with_context(|| format!("no key at {}..{}", position, equal_sign_index))?;
            let value = query_string
                .get(equal_sign_index + 1..segment_end)
                .with_context(|| {
                    format!("no value at {}..{}", equal_sign_index + 1, segment_end)
                })?;

            match key {
                "info_hash" => {
                    let value = urldecode_20_bytes(value)?;

                    info_hashes.push(InfoHash(value));
                }
                k => {
                    ::log::debug!("ignored unrecognized key: {}", k)
                }
            }

            if segment_end == query_string.len() {
                break;
            } else {
                position = segment_end + 1;
            }
        }

        if info_hashes.is_empty() {
            return Err(anyhow::anyhow!("No info hashes sent"));
        }

        Ok(ScrapeRequest { info_hashes })
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Request {
    Announce(AnnounceRequest),
    Scrape(ScrapeRequest),
}

impl Request {
    /// Parse Request from HTTP request bytes
    pub fn parse_bytes(bytes: &[u8]) -> anyhow::Result<Option<Self>> {
        let mut headers = [httparse::EMPTY_HEADER; 16];
        let mut http_request = httparse::Request::new(&mut headers);

        match http_request.parse(bytes) {
            Ok(httparse::Status::Complete(_)) => {
                if let Some(path) = http_request.path {
                    Self::parse_http_get_path(path).map(Some)
                } else {
                    Err(anyhow::anyhow!("no http path"))
                }
            }
            Ok(httparse::Status::Partial) => Ok(None),
            Err(err) => Err(anyhow::Error::from(err)),
        }
    }

    /// Parse Request from http GET path (`/announce?info_hash=...`)
    ///
    /// Existing serde-url decode crates were insufficient, so the decision was
    /// made to create a custom parser. serde_urlencoded doesn't support multiple
    /// values with same key, and serde_qs pulls in lots of dependencies. Both
    /// would need preprocessing for the binary format used for info_hash and
    /// peer_id.
    ///
    /// The info hashes and peer id's that are received are url-encoded byte
    /// by byte, e.g., %fa for byte 0xfa. However, they need to be parsed as
    /// UTF-8 string, meaning that non-ascii bytes are invalid characters.
    /// Therefore, these bytes must be converted to their equivalent multi-byte
    /// UTF-8 encodings.
    pub fn parse_http_get_path(path: &str) -> anyhow::Result<Self> {
        ::log::debug!("request GET path: {}", path);

        let mut split_parts = path.splitn(2, '?');

        let location = split_parts.next().with_context(|| "no location")?;
        let query_string = split_parts.next().with_context(|| "no query string")?;

        if location == "/announce" {
            Ok(Request::Announce(AnnounceRequest::parse_query_string(
                query_string,
            )?))
        } else if location == "/scrape" {
            Ok(Request::Scrape(ScrapeRequest::parse_query_string(
                query_string,
            )?))
        } else {
            Err(anyhow::anyhow!("Path must be /announce or /scrape"))
        }
    }

    pub fn write<W: Write>(&self, output: &mut W, url_suffix: &[u8]) -> ::std::io::Result<()> {
        match self {
            Self::Announce(r) => r.write_bytes(output, url_suffix),
            Self::Scrape(r) => r.write_bytes(output, url_suffix),
        }
    }
}

#[cfg(test)]
mod tests {
    use quickcheck::{quickcheck, Arbitrary, Gen, TestResult};

    use super::*;

    static ANNOUNCE_REQUEST_PATH: &str = "/announce?info_hash=%04%0bkV%3f%5cr%14%a6%b7%98%adC%c3%c9.%40%24%00%b9&peer_id=-ABC940-5ert69muw5t8&port=12345&uploaded=1&downloaded=2&left=3&numwant=0&key=4ab4b877&compact=1&supportcrypto=1&event=started";
    static SCRAPE_REQUEST_PATH: &str =
        "/scrape?info_hash=%04%0bkV%3f%5cr%14%a6%b7%98%adC%c3%c9.%40%24%00%b9";
    static REFERENCE_INFO_HASH: [u8; 20] = [
        0x04, 0x0b, b'k', b'V', 0x3f, 0x5c, b'r', 0x14, 0xa6, 0xb7, 0x98, 0xad, b'C', 0xc3, 0xc9,
        b'.', 0x40, 0x24, 0x00, 0xb9,
    ];
    static REFERENCE_PEER_ID: [u8; 20] = [
        b'-', b'A', b'B', b'C', b'9', b'4', b'0', b'-', b'5', b'e', b'r', b't', b'6', b'9', b'm',
        b'u', b'w', b'5', b't', b'8',
    ];

    fn get_reference_announce_request() -> Request {
        Request::Announce(AnnounceRequest {
            info_hash: InfoHash(REFERENCE_INFO_HASH),
            peer_id: PeerId(REFERENCE_PEER_ID),
            port: 12345,
            bytes_uploaded: 1,
            bytes_downloaded: 2,
            bytes_left: 3,
            event: AnnounceEvent::Started,
            numwant: Some(0),
            key: Some("4ab4b877".into()),
        })
    }

    #[test]
    fn test_announce_request_from_bytes() {
        let mut bytes = Vec::new();

        bytes.extend_from_slice(b"GET ");
        bytes.extend_from_slice(ANNOUNCE_REQUEST_PATH.as_bytes());
        bytes.extend_from_slice(b" HTTP/1.1\r\n\r\n");

        let parsed_request = Request::parse_bytes(&bytes[..]).unwrap().unwrap();
        let reference_request = get_reference_announce_request();

        assert_eq!(parsed_request, reference_request);
    }

    #[test]
    fn test_scrape_request_from_bytes() {
        let mut bytes = Vec::new();

        bytes.extend_from_slice(b"GET ");
        bytes.extend_from_slice(SCRAPE_REQUEST_PATH.as_bytes());
        bytes.extend_from_slice(b" HTTP/1.1\r\n\r\n");

        let parsed_request = Request::parse_bytes(&bytes[..]).unwrap().unwrap();
        let reference_request = Request::Scrape(ScrapeRequest {
            info_hashes: vec![InfoHash(REFERENCE_INFO_HASH)],
        });

        assert_eq!(parsed_request, reference_request);
    }

    impl Arbitrary for AnnounceRequest {
        fn arbitrary(g: &mut Gen) -> Self {
            let key: Option<String> = Arbitrary::arbitrary(g);

            AnnounceRequest {
                info_hash: Arbitrary::arbitrary(g),
                peer_id: Arbitrary::arbitrary(g),
                port: Arbitrary::arbitrary(g),
                bytes_uploaded: Arbitrary::arbitrary(g),
                bytes_downloaded: Arbitrary::arbitrary(g),
                bytes_left: Arbitrary::arbitrary(g),
                event: Arbitrary::arbitrary(g),
                numwant: Arbitrary::arbitrary(g),
                key: key.map(|key| key.into()),
            }
        }
    }

    impl Arbitrary for ScrapeRequest {
        fn arbitrary(g: &mut Gen) -> Self {
            ScrapeRequest {
                info_hashes: Arbitrary::arbitrary(g),
            }
        }
    }

    impl Arbitrary for Request {
        fn arbitrary(g: &mut Gen) -> Self {
            if Arbitrary::arbitrary(g) {
                Self::Announce(Arbitrary::arbitrary(g))
            } else {
                Self::Scrape(Arbitrary::arbitrary(g))
            }
        }
    }

    #[test]
    fn quickcheck_serde_identity_request() {
        fn prop(request: Request) -> TestResult {
            match request {
                Request::Announce(AnnounceRequest {
                    key: Some(ref key), ..
                }) => {
                    if key.len() > 30 {
                        return TestResult::discard();
                    }
                }
                Request::Scrape(ScrapeRequest { ref info_hashes }) => {
                    if info_hashes.is_empty() {
                        return TestResult::discard();
                    }
                }
                _ => {}
            }

            let mut bytes = Vec::new();

            request.write(&mut bytes, &[]).unwrap();

            let parsed_request = Request::parse_bytes(&bytes[..]).unwrap().unwrap();

            let success = request == parsed_request;

            if !success {
                println!("request:        {:?}", request);
                println!("parsed request: {:?}", parsed_request);
                println!("bytes as str:   {}", String::from_utf8_lossy(&bytes));
            }

            TestResult::from_bool(success)
        }

        quickcheck(prop as fn(Request) -> TestResult);
    }
}
