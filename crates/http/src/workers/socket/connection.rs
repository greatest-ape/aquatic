use std::cell::RefCell;
use std::collections::BTreeMap;
use std::net::SocketAddr;
use std::rc::Rc;
use std::sync::Arc;

use anyhow::Context;
use aquatic_common::access_list::{create_access_list_cache, AccessListArcSwap, AccessListCache};
use aquatic_common::rustls_config::RustlsConfig;
use aquatic_common::{CanonicalSocketAddr, ServerStartInstant};
use aquatic_http_protocol::common::InfoHash;
use aquatic_http_protocol::request::{Request, ScrapeRequest};
use aquatic_http_protocol::response::{
    FailureResponse, Response, ScrapeResponse, ScrapeStatistics,
};
use arc_swap::ArcSwap;
use either::Either;
use futures::stream::FuturesUnordered;
use futures_lite::{AsyncReadExt, AsyncWriteExt, StreamExt};
use futures_rustls::TlsAcceptor;
use glommio::channels::channel_mesh::Senders;
use glommio::channels::shared_channel::{self, SharedReceiver};
use glommio::net::TcpStream;
use once_cell::sync::Lazy;

use crate::common::*;
use crate::config::Config;

#[cfg(feature = "metrics")]
use super::peer_addr_to_ip_version_str;
use super::request::{parse_request, RequestParseError};

const REQUEST_BUFFER_SIZE: usize = 2048;
const RESPONSE_BUFFER_SIZE: usize = 4096;

const RESPONSE_HEADER_A: &[u8] = b"HTTP/1.1 200 OK\r\nContent-Length: ";
const RESPONSE_HEADER_B: &[u8] = b"        ";
const RESPONSE_HEADER_C: &[u8] = b"\r\n\r\n";

static RESPONSE_HEADER: Lazy<Vec<u8>> =
    Lazy::new(|| [RESPONSE_HEADER_A, RESPONSE_HEADER_B, RESPONSE_HEADER_C].concat());

struct PendingScrapeResponse {
    pending_worker_responses: usize,
    stats: BTreeMap<InfoHash, ScrapeStatistics>,
}

#[derive(Debug, thiserror::Error)]
pub enum ConnectionError {
    #[error("inactive")]
    Inactive,
    #[error("socket peer addr extraction failed")]
    NoSocketPeerAddr(String),
    #[error("request buffer full")]
    RequestBufferFull,
    #[error("response buffer full")]
    ResponseBufferFull,
    #[error("response buffer write error: {0}")]
    ResponseBufferWrite(::std::io::Error),
    #[error("peer closed")]
    PeerClosed,
    #[error("response sender closed")]
    ResponseSenderClosed,
    #[error("scrape channel error: {0}")]
    ScrapeChannelError(&'static str),
    #[error(transparent)]
    Other(#[from] anyhow::Error),
}

#[allow(clippy::too_many_arguments)]
pub(super) async fn run_connection(
    config: Rc<Config>,
    access_list: Arc<AccessListArcSwap>,
    request_senders: Rc<Senders<ChannelRequest>>,
    server_start_instant: ServerStartInstant,
    opt_tls_config: Option<Arc<ArcSwap<RustlsConfig>>>,
    valid_until: Rc<RefCell<ValidUntil>>,
    stream: TcpStream,
    worker_index: usize,
) -> Result<(), ConnectionError> {
    let access_list_cache = create_access_list_cache(&access_list);
    let request_buffer = Box::new([0u8; REQUEST_BUFFER_SIZE]);

    let mut response_buffer = Box::new([0; RESPONSE_BUFFER_SIZE]);

    response_buffer[..RESPONSE_HEADER.len()].copy_from_slice(&RESPONSE_HEADER);

    let remote_addr = stream
        .peer_addr()
        .map_err(|err| ConnectionError::NoSocketPeerAddr(err.to_string()))?;

    let opt_peer_addr = if config.network.runs_behind_reverse_proxy {
        None
    } else {
        Some(CanonicalSocketAddr::new(remote_addr))
    };

    let peer_port = remote_addr.port();

    if let Some(tls_config) = opt_tls_config {
        let tls_acceptor: TlsAcceptor = tls_config.load_full().into();
        let stream = tls_acceptor
            .accept(stream)
            .await
            .with_context(|| "tls accept")?;

        let mut conn = Connection {
            config,
            access_list_cache,
            request_senders,
            valid_until,
            server_start_instant,
            opt_peer_addr,
            peer_port,
            request_buffer,
            request_buffer_position: 0,
            response_buffer,
            stream,
            worker_index_string: worker_index.to_string(),
        };

        conn.run().await
    } else {
        let mut conn = Connection {
            config,
            access_list_cache,
            request_senders,
            valid_until,
            server_start_instant,
            opt_peer_addr,
            peer_port,
            request_buffer,
            request_buffer_position: 0,
            response_buffer,
            stream,
            worker_index_string: worker_index.to_string(),
        };

        conn.run().await
    }
}

struct Connection<S> {
    config: Rc<Config>,
    access_list_cache: AccessListCache,
    request_senders: Rc<Senders<ChannelRequest>>,
    valid_until: Rc<RefCell<ValidUntil>>,
    server_start_instant: ServerStartInstant,
    opt_peer_addr: Option<CanonicalSocketAddr>,
    peer_port: u16,
    request_buffer: Box<[u8; REQUEST_BUFFER_SIZE]>,
    request_buffer_position: usize,
    response_buffer: Box<[u8; RESPONSE_BUFFER_SIZE]>,
    stream: S,
    worker_index_string: String,
}

impl<S> Connection<S>
where
    S: futures::AsyncRead + futures::AsyncWrite + Unpin + 'static,
{
    async fn run(&mut self) -> Result<(), ConnectionError> {
        loop {
            let response = match self.read_request().await? {
                Either::Left(response) => Response::Failure(response),
                Either::Right(request) => self.handle_request(request).await?,
            };

            self.write_response(&response).await?;

            if matches!(response, Response::Failure(_)) || !self.config.network.keep_alive {
                break;
            }
        }

        Ok(())
    }

    async fn read_request(&mut self) -> Result<Either<FailureResponse, Request>, ConnectionError> {
        self.request_buffer_position = 0;

        loop {
            if self.request_buffer_position == self.request_buffer.len() {
                return Err(ConnectionError::RequestBufferFull);
            }

            let bytes_read = self
                .stream
                .read(&mut self.request_buffer[self.request_buffer_position..])
                .await
                .with_context(|| "read")?;

            if bytes_read == 0 {
                return Err(ConnectionError::PeerClosed);
            }

            self.request_buffer_position += bytes_read;

            let buffer_slice = &self.request_buffer[..self.request_buffer_position];

            match parse_request(&self.config, buffer_slice) {
                Ok((request, opt_peer_ip)) => {
                    if self.config.network.runs_behind_reverse_proxy {
                        let peer_ip = opt_peer_ip
                            .expect("logic error: peer ip must have been extracted at this point");

                        self.opt_peer_addr = Some(CanonicalSocketAddr::new(SocketAddr::new(
                            peer_ip,
                            self.peer_port,
                        )));
                    }

                    return Ok(Either::Right(request));
                }
                Err(RequestParseError::MoreDataNeeded) => continue,
                Err(RequestParseError::RequiredPeerIpHeaderMissing(err)) => {
                    panic!("Tracker configured as running behind reverse proxy, but no corresponding IP header set in request. Please check your reverse proxy setup as well as your aquatic configuration. Error: {:#}", err);
                }
                Err(RequestParseError::Other(err)) => {
                    ::log::debug!("Failed parsing request: {:#}", err);

                    let response = FailureResponse {
                        failure_reason: "Invalid request".into(),
                    };

                    return Ok(Either::Left(response));
                }
            }
        }
    }

    /// Take a request and:
    /// - Update connection ValidUntil
    /// - Return error response if request is not allowed
    /// - If it is an announce request, send it to swarm workers an await a
    ///   response
    /// - If it is a scrape requests, split it up, pass on the parts to
    ///   relevant swarm workers and await a response
    async fn handle_request(&mut self, request: Request) -> Result<Response, ConnectionError> {
        let peer_addr = self
            .opt_peer_addr
            .expect("peer addr should already have been extracted by now");

        *self.valid_until.borrow_mut() = ValidUntil::new(
            self.server_start_instant,
            self.config.cleaning.max_connection_idle,
        );

        match request {
            Request::Announce(request) => {
                #[cfg(feature = "metrics")]
                ::metrics::counter!(
                    "aquatic_requests_total",
                    "type" => "announce",
                    "ip_version" => peer_addr_to_ip_version_str(&peer_addr),
                    "worker_index" => self.worker_index_string.clone(),
                )
                .increment(1);

                let info_hash = request.info_hash;

                if self
                    .access_list_cache
                    .load()
                    .allows(self.config.access_list.mode, &info_hash.0)
                {
                    let (response_sender, response_receiver) = shared_channel::new_bounded(1);

                    let request = ChannelRequest::Announce {
                        request,
                        peer_addr,
                        response_sender,
                    };

                    let consumer_index = calculate_request_consumer_index(&self.config, info_hash);

                    // Only fails when receiver is closed
                    self.request_senders
                        .send_to(consumer_index, request)
                        .await
                        .unwrap();

                    response_receiver
                        .connect()
                        .await
                        .recv()
                        .await
                        .ok_or(ConnectionError::ResponseSenderClosed)
                        .map(Response::Announce)
                } else {
                    let response = Response::Failure(FailureResponse {
                        failure_reason: "Info hash not allowed".into(),
                    });

                    Ok(response)
                }
            }
            Request::Scrape(ScrapeRequest { info_hashes }) => {
                #[cfg(feature = "metrics")]
                ::metrics::counter!(
                    "aquatic_requests_total",
                    "type" => "scrape",
                    "ip_version" => peer_addr_to_ip_version_str(&peer_addr),
                    "worker_index" => self.worker_index_string.clone(),
                )
                .increment(1);

                let mut info_hashes_by_worker: BTreeMap<usize, Vec<InfoHash>> = BTreeMap::new();

                for info_hash in info_hashes.into_iter() {
                    let info_hashes = info_hashes_by_worker
                        .entry(calculate_request_consumer_index(&self.config, info_hash))
                        .or_default();

                    info_hashes.push(info_hash);
                }

                let pending_worker_responses = info_hashes_by_worker.len();
                let mut response_receivers = Vec::with_capacity(pending_worker_responses);

                for (consumer_index, info_hashes) in info_hashes_by_worker {
                    let (response_sender, response_receiver) = shared_channel::new_bounded(1);

                    response_receivers.push(response_receiver);

                    let request = ChannelRequest::Scrape {
                        request: ScrapeRequest { info_hashes },
                        peer_addr,
                        response_sender,
                    };

                    // Only fails when receiver is closed
                    self.request_senders
                        .send_to(consumer_index, request)
                        .await
                        .unwrap();
                }

                let pending_scrape_response = PendingScrapeResponse {
                    pending_worker_responses,
                    stats: Default::default(),
                };

                self.wait_for_scrape_responses(response_receivers, pending_scrape_response)
                    .await
            }
        }
    }

    /// Wait for partial scrape responses to arrive,
    /// return full response
    async fn wait_for_scrape_responses(
        &self,
        response_receivers: Vec<SharedReceiver<ScrapeResponse>>,
        mut pending: PendingScrapeResponse,
    ) -> Result<Response, ConnectionError> {
        let mut responses = response_receivers
            .into_iter()
            .map(|receiver| async { receiver.connect().await.recv().await })
            .collect::<FuturesUnordered<_>>();

        loop {
            let response = responses
                .next()
                .await
                .ok_or_else(|| {
                    ConnectionError::ScrapeChannelError(
                        "stream ended before all partial scrape responses received",
                    )
                })?
                .ok_or_else(|| ConnectionError::ScrapeChannelError("sender is closed"))?;

            pending.stats.extend(response.files);
            pending.pending_worker_responses -= 1;

            if pending.pending_worker_responses == 0 {
                let response = Response::Scrape(ScrapeResponse {
                    files: pending.stats,
                });

                break Ok(response);
            }
        }
    }

    async fn write_response(&mut self, response: &Response) -> Result<(), ConnectionError> {
        // Write body and final newline to response buffer

        let mut position = RESPONSE_HEADER.len();

        let body_len = response
            .write_bytes(&mut &mut self.response_buffer[position..])
            .map_err(ConnectionError::ResponseBufferWrite)?;

        position += body_len;

        if position + 2 > self.response_buffer.len() {
            return Err(ConnectionError::ResponseBufferFull);
        }

        self.response_buffer[position..position + 2].copy_from_slice(b"\r\n");

        position += 2;

        let content_len = body_len + 2;

        // Clear content-len header value

        {
            let start = RESPONSE_HEADER_A.len();
            let end = start + RESPONSE_HEADER_B.len();

            self.response_buffer[start..end].copy_from_slice(RESPONSE_HEADER_B);
        }

        // Set content-len header value

        {
            let mut buf = ::itoa::Buffer::new();
            let content_len_bytes = buf.format(content_len).as_bytes();

            let start = RESPONSE_HEADER_A.len();
            let end = start + content_len_bytes.len();

            self.response_buffer[start..end].copy_from_slice(content_len_bytes);
        }

        // Write buffer to stream

        self.stream
            .write(&self.response_buffer[..position])
            .await
            .with_context(|| "write")?;
        self.stream.flush().await.with_context(|| "flush")?;

        #[cfg(feature = "metrics")]
        {
            let response_type = match response {
                Response::Announce(_) => "announce",
                Response::Scrape(_) => "scrape",
                Response::Failure(_) => "error",
            };

            // If we're behind a reverse proxy and we're sending an error
            // response due to failing to parse a request, opt_peer_addr might
            // not yet be set (and in that case, we don't know if the true peer
            // connects over IPv4 or IPv6)
            let ip_version_str = self
                .opt_peer_addr
                .as_ref()
                .map(peer_addr_to_ip_version_str)
                .unwrap_or("?");

            ::metrics::counter!(
                "aquatic_responses_total",
                "type" => response_type,
                "ip_version" => ip_version_str,
                "worker_index" => self.worker_index_string.clone(),
            )
            .increment(1);
        }

        Ok(())
    }
}

fn calculate_request_consumer_index(config: &Config, info_hash: InfoHash) -> usize {
    (info_hash.0[0] as usize) % config.swarm_workers
}
