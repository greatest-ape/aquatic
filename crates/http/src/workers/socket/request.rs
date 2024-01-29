use std::net::IpAddr;

use anyhow::Context;
use aquatic_http_protocol::request::Request;

use crate::config::{Config, ReverseProxyPeerIpHeaderFormat};

#[derive(Debug, thiserror::Error)]
pub enum RequestParseError {
    #[error("required peer ip header missing or invalid")]
    RequiredPeerIpHeaderMissing(anyhow::Error),
    #[error("more data needed")]
    MoreDataNeeded,
    #[error(transparent)]
    Other(#[from] anyhow::Error),
}

pub fn parse_request(
    config: &Config,
    buffer: &[u8],
) -> Result<(Request, Option<IpAddr>), RequestParseError> {
    let mut headers = [httparse::EMPTY_HEADER; 16];
    let mut http_request = httparse::Request::new(&mut headers);

    match http_request.parse(buffer).with_context(|| "httparse")? {
        httparse::Status::Complete(_) => {
            let path = http_request.path.ok_or(anyhow::anyhow!("no http path"))?;
            let request = Request::parse_http_get_path(path)?;

            let opt_peer_ip = if config.network.runs_behind_reverse_proxy {
                let header_name = &config.network.reverse_proxy_ip_header_name;
                let header_format = config.network.reverse_proxy_ip_header_format;

                match parse_forwarded_header(header_name, header_format, http_request.headers) {
                    Ok(peer_ip) => Some(peer_ip),
                    Err(err) => {
                        return Err(RequestParseError::RequiredPeerIpHeaderMissing(err));
                    }
                }
            } else {
                None
            };

            Ok((request, opt_peer_ip))
        }
        httparse::Status::Partial => Err(RequestParseError::MoreDataNeeded),
    }
}

fn parse_forwarded_header(
    header_name: &str,
    header_format: ReverseProxyPeerIpHeaderFormat,
    headers: &[httparse::Header<'_>],
) -> anyhow::Result<IpAddr> {
    for header in headers.iter().rev() {
        if header.name == header_name {
            match header_format {
                ReverseProxyPeerIpHeaderFormat::LastAddress => {
                    return ::std::str::from_utf8(header.value)?
                        .split(',')
                        .last()
                        .ok_or(anyhow::anyhow!("no header value"))?
                        .trim()
                        .parse::<IpAddr>()
                        .with_context(|| "parse ip");
                }
            }
        }
    }

    Err(anyhow::anyhow!("header not present"))
}

#[cfg(test)]
mod tests {
    use super::*;

    const REQUEST_START: &str = "GET /announce?info_hash=%04%0bkV%3f%5cr%14%a6%b7%98%adC%c3%c9.%40%24%00%b9&peer_id=-ABC940-5ert69muw5t8&port=12345&uploaded=1&downloaded=2&left=3&numwant=0&key=4ab4b877&compact=1&supportcrypto=1&event=started HTTP/1.1\r\nHost: example.com\r\n";

    #[test]
    fn test_parse_peer_ip_header_multiple() {
        let mut config = Config::default();

        config.network.runs_behind_reverse_proxy = true;
        config.network.reverse_proxy_ip_header_name = "X-Forwarded-For".into();
        config.network.reverse_proxy_ip_header_format = ReverseProxyPeerIpHeaderFormat::LastAddress;

        let mut request = REQUEST_START.to_string();

        request.push_str("X-Forwarded-For: 200.0.0.1\r\n");
        request.push_str("X-Forwarded-For: 1.2.3.4, 5.6.7.8,9.10.11.12\r\n");
        request.push_str("\r\n");

        let expected_ip = IpAddr::from([9, 10, 11, 12]);

        assert_eq!(
            parse_request(&config, request.as_bytes())
                .unwrap()
                .1
                .unwrap(),
            expected_ip
        )
    }

    #[test]
    fn test_parse_peer_ip_header_single() {
        let mut config = Config::default();

        config.network.runs_behind_reverse_proxy = true;
        config.network.reverse_proxy_ip_header_name = "X-Forwarded-For".into();
        config.network.reverse_proxy_ip_header_format = ReverseProxyPeerIpHeaderFormat::LastAddress;

        let mut request = REQUEST_START.to_string();

        request.push_str("X-Forwarded-For: 1.2.3.4, 5.6.7.8,9.10.11.12\r\n");
        request.push_str("X-Forwarded-For: 200.0.0.1\r\n");
        request.push_str("\r\n");

        let expected_ip = IpAddr::from([200, 0, 0, 1]);

        assert_eq!(
            parse_request(&config, request.as_bytes())
                .unwrap()
                .1
                .unwrap(),
            expected_ip
        )
    }

    #[test]
    fn test_parse_peer_ip_header_no_header() {
        let mut config = Config::default();

        config.network.runs_behind_reverse_proxy = true;

        let mut request = REQUEST_START.to_string();

        request.push_str("\r\n");

        let res = parse_request(&config, request.as_bytes());

        assert!(matches!(
            res,
            Err(RequestParseError::RequiredPeerIpHeaderMissing(_))
        ));
    }
}
