use std::cell::RefCell;
use std::collections::BTreeMap;
use std::os::unix::prelude::{FromRawFd, IntoRawFd};
use std::rc::Rc;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};

use aquatic_common::access_list::{create_access_list_cache, AccessListArcSwap, AccessListCache};
use aquatic_common::CanonicalSocketAddr;
use aquatic_http_protocol::common::InfoHash;
use aquatic_http_protocol::request::{Request, RequestParseError, ScrapeRequest};
use aquatic_http_protocol::response::{
    FailureResponse, Response, ScrapeResponse, ScrapeStatistics,
};
use either::Either;
use futures_lite::{AsyncReadExt, AsyncWriteExt, StreamExt};
use futures_rustls::server::TlsStream;
use futures_rustls::TlsAcceptor;
use glommio::channels::channel_mesh::{MeshBuilder, Partial, Role, Senders};
use glommio::channels::local_channel::{new_bounded, LocalReceiver, LocalSender};
use glommio::channels::shared_channel::ConnectedReceiver;
use glommio::net::{TcpListener, TcpStream};
use glommio::task::JoinHandle;
use glommio::timer::TimerActionRepeat;
use glommio::{enclose, prelude::*};
use once_cell::sync::Lazy;
use slab::Slab;

use crate::common::*;
use crate::config::Config;

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

struct ConnectionReference {
    task_handle: Option<JoinHandle<()>>,
    response_sender: LocalSender<ChannelResponse>,
    valid_until: ValidUntil,
}

pub async fn run_socket_worker(
    config: Config,
    state: State,
    tls_config: Arc<TlsConfig>,
    request_mesh_builder: MeshBuilder<ChannelRequest, Partial>,
    response_mesh_builder: MeshBuilder<ChannelResponse, Partial>,
    num_bound_sockets: Arc<AtomicUsize>,
) {
    let config = Rc::new(config);
    let access_list = state.access_list;

    let listener = create_tcp_listener(&config);
    num_bound_sockets.fetch_add(1, Ordering::SeqCst);

    let (request_senders, _) = request_mesh_builder.join(Role::Producer).await.unwrap();
    let request_senders = Rc::new(request_senders);

    let (_, mut response_receivers) = response_mesh_builder.join(Role::Consumer).await.unwrap();
    let response_consumer_id = ConsumerId(response_receivers.consumer_id().unwrap());

    let connection_slab = Rc::new(RefCell::new(Slab::new()));

    TimerActionRepeat::repeat(enclose!((config, connection_slab) move || {
        clean_connections(
            config.clone(),
            connection_slab.clone(),
        )
    }));

    for (_, response_receiver) in response_receivers.streams() {
        spawn_local(receive_responses(
            response_receiver,
            connection_slab.clone(),
        ))
        .detach();
    }

    let mut incoming = listener.incoming();

    while let Some(stream) = incoming.next().await {
        match stream {
            Ok(stream) => {
                let (response_sender, response_receiver) = new_bounded(config.request_workers);

                let key = connection_slab.borrow_mut().insert(ConnectionReference {
                    task_handle: None,
                    response_sender,
                    valid_until: ValidUntil::new(config.cleaning.max_connection_idle),
                });

                let task_handle = spawn_local(enclose!((config, access_list, request_senders, tls_config, connection_slab) async move {
                    if let Err(err) = Connection::run(
                        config,
                        access_list,
                        request_senders,
                        response_receiver,
                        response_consumer_id,
                        ConnectionId(key),
                        tls_config,
                        connection_slab.clone(),
                        stream
                    ).await {
                        ::log::debug!("Connection::run() error: {:?}", err);
                    }

                    connection_slab.borrow_mut().try_remove(key);
                }))
                .detach();

                if let Some(reference) = connection_slab.borrow_mut().get_mut(key) {
                    reference.task_handle = Some(task_handle);
                }
            }
            Err(err) => {
                ::log::error!("accept connection: {:?}", err);
            }
        }
    }
}

async fn clean_connections(
    config: Rc<Config>,
    connection_slab: Rc<RefCell<Slab<ConnectionReference>>>,
) -> Option<Duration> {
    let now = Instant::now();

    connection_slab.borrow_mut().retain(|_, reference| {
        if reference.valid_until.0 > now {
            true
        } else {
            if let Some(ref handle) = reference.task_handle {
                handle.cancel();
            }

            false
        }
    });

    connection_slab.borrow_mut().shrink_to_fit();

    Some(Duration::from_secs(
        config.cleaning.connection_cleaning_interval,
    ))
}

async fn receive_responses(
    mut response_receiver: ConnectedReceiver<ChannelResponse>,
    connection_references: Rc<RefCell<Slab<ConnectionReference>>>,
) {
    while let Some(channel_response) = response_receiver.next().await {
        if let Some(reference) = connection_references
            .borrow()
            .get(channel_response.get_connection_id().0)
        {
            if let Err(err) = reference.response_sender.try_send(channel_response) {
                ::log::error!("Couldn't send response to local receiver: {:?}", err);
            }
        }
    }
}

struct Connection {
    config: Rc<Config>,
    access_list_cache: AccessListCache,
    request_senders: Rc<Senders<ChannelRequest>>,
    response_receiver: LocalReceiver<ChannelResponse>,
    response_consumer_id: ConsumerId,
    connection_slab: Rc<RefCell<Slab<ConnectionReference>>>,
    stream: TlsStream<TcpStream>,
    peer_addr: CanonicalSocketAddr,
    connection_id: ConnectionId,
    request_buffer: [u8; REQUEST_BUFFER_SIZE],
    request_buffer_position: usize,
    response_buffer: [u8; RESPONSE_BUFFER_SIZE],
}

impl Connection {
    async fn run(
        config: Rc<Config>,
        access_list: Arc<AccessListArcSwap>,
        request_senders: Rc<Senders<ChannelRequest>>,
        response_receiver: LocalReceiver<ChannelResponse>,
        response_consumer_id: ConsumerId,
        connection_id: ConnectionId,
        tls_config: Arc<TlsConfig>,
        connection_slab: Rc<RefCell<Slab<ConnectionReference>>>,
        stream: TcpStream,
    ) -> anyhow::Result<()> {
        let peer_addr = stream
            .peer_addr()
            .map_err(|err| anyhow::anyhow!("Couldn't get peer addr: {:?}", err))?;
        let peer_addr = CanonicalSocketAddr::new(peer_addr);

        let tls_acceptor: TlsAcceptor = tls_config.into();
        let stream = tls_acceptor.accept(stream).await?;

        let mut response_buffer = [0; RESPONSE_BUFFER_SIZE];

        response_buffer[..RESPONSE_HEADER.len()].copy_from_slice(&RESPONSE_HEADER);

        let mut conn = Connection {
            config: config.clone(),
            access_list_cache: create_access_list_cache(&access_list),
            request_senders: request_senders.clone(),
            response_receiver,
            response_consumer_id,
            connection_slab,
            stream,
            peer_addr,
            connection_id,
            request_buffer: [0; REQUEST_BUFFER_SIZE],
            request_buffer_position: 0,
            response_buffer,
        };

        conn.run_request_response_loop().await?;

        Ok(())
    }

    async fn run_request_response_loop(&mut self) -> anyhow::Result<()> {
        loop {
            let response = match self.read_request().await? {
                Either::Left(response) => Response::Failure(response),
                Either::Right(request) => self.handle_request(request).await?,
            };

            self.write_response(&response).await?;

            if matches!(response, Response::Failure(_)) || !self.config.network.keep_alive {
                let _ = self
                    .stream
                    .get_ref()
                    .0
                    .shutdown(std::net::Shutdown::Both)
                    .await;

                break;
            }
        }

        Ok(())
    }

    async fn read_request(&mut self) -> anyhow::Result<Either<FailureResponse, Request>> {
        self.request_buffer_position = 0;

        loop {
            if self.request_buffer_position == self.request_buffer.len() {
                return Err(anyhow::anyhow!("request buffer is full"));
            }

            let bytes_read = self
                .stream
                .read(&mut self.request_buffer[self.request_buffer_position..])
                .await?;

            if bytes_read == 0 {
                return Err(anyhow::anyhow!("peer closed connection"));
            }

            self.request_buffer_position += bytes_read;

            match Request::from_bytes(&self.request_buffer[..self.request_buffer_position]) {
                Ok(request) => {
                    ::log::debug!("received request: {:?}", request);

                    return Ok(Either::Right(request));
                }
                Err(RequestParseError::Invalid(err)) => {
                    ::log::debug!("invalid request: {:?}", err);

                    let response = FailureResponse {
                        failure_reason: "Invalid request".into(),
                    };

                    return Ok(Either::Left(response));
                }
                Err(RequestParseError::NeedMoreData) => {
                    ::log::debug!(
                        "need more request data. current data: {:?}",
                        std::str::from_utf8(&self.request_buffer[..self.request_buffer_position])
                    );
                }
            }
        }
    }

    /// Take a request and:
    /// - Update connection ValidUntil
    /// - Return error response if request is not allowed
    /// - If it is an announce request, send it to request workers an await a
    ///   response
    /// - If it is a scrape requests, split it up, pass on the parts to
    ///   relevant request workers and await a response
    async fn handle_request(&mut self, request: Request) -> anyhow::Result<Response> {
        if let Ok(mut slab) = self.connection_slab.try_borrow_mut() {
            if let Some(reference) = slab.get_mut(self.connection_id.0) {
                reference.valid_until = ValidUntil::new(self.config.cleaning.max_connection_idle);
            }
        }

        match request {
            Request::Announce(request) => {
                let info_hash = request.info_hash;

                if self
                    .access_list_cache
                    .load()
                    .allows(self.config.access_list.mode, &info_hash.0)
                {
                    let request = ChannelRequest::Announce {
                        request,
                        connection_id: self.connection_id,
                        response_consumer_id: self.response_consumer_id,
                        peer_addr: self.peer_addr,
                    };

                    let consumer_index = calculate_request_consumer_index(&self.config, info_hash);

                    // Only fails when receiver is closed
                    self.request_senders
                        .send_to(consumer_index, request)
                        .await
                        .unwrap();

                    self.wait_for_response(None).await
                } else {
                    let response = Response::Failure(FailureResponse {
                        failure_reason: "Info hash not allowed".into(),
                    });

                    Ok(response)
                }
            }
            Request::Scrape(ScrapeRequest { info_hashes }) => {
                let mut info_hashes_by_worker: BTreeMap<usize, Vec<InfoHash>> = BTreeMap::new();

                for info_hash in info_hashes.into_iter() {
                    let info_hashes = info_hashes_by_worker
                        .entry(calculate_request_consumer_index(&self.config, info_hash))
                        .or_default();

                    info_hashes.push(info_hash);
                }

                let pending_worker_responses = info_hashes_by_worker.len();

                for (consumer_index, info_hashes) in info_hashes_by_worker {
                    let request = ChannelRequest::Scrape {
                        request: ScrapeRequest { info_hashes },
                        peer_addr: self.peer_addr,
                        response_consumer_id: self.response_consumer_id,
                        connection_id: self.connection_id,
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

                self.wait_for_response(Some(pending_scrape_response)).await
            }
        }
    }

    /// Wait for announce response or partial scrape responses to arrive,
    /// return full response
    async fn wait_for_response(
        &self,
        mut opt_pending_scrape_response: Option<PendingScrapeResponse>,
    ) -> anyhow::Result<Response> {
        loop {
            let channel_response = self
                .response_receiver
                .recv()
                .await
                .expect("wait_for_response: can't receive response, sender is closed");

            if channel_response.get_peer_addr() != self.peer_addr {
                return Err(anyhow::anyhow!("peer addresses didn't match"));
            }

            match channel_response {
                ChannelResponse::Announce { response, .. } => {
                    break Ok(Response::Announce(response));
                }
                ChannelResponse::Scrape { response, .. } => {
                    if let Some(mut pending) = opt_pending_scrape_response.take() {
                        pending.stats.extend(response.files);
                        pending.pending_worker_responses -= 1;

                        if pending.pending_worker_responses == 0 {
                            let response = Response::Scrape(ScrapeResponse {
                                files: pending.stats,
                            });

                            break Ok(response);
                        } else {
                            opt_pending_scrape_response = Some(pending);
                        }
                    } else {
                        return Err(anyhow::anyhow!(
                            "received channel scrape response without pending scrape response"
                        ));
                    }
                }
            };
        }
    }

    async fn write_response(&mut self, response: &Response) -> anyhow::Result<()> {
        // Write body and final newline to response buffer

        let mut position = RESPONSE_HEADER.len();

        let body_len = response.write(&mut &mut self.response_buffer[position..])?;

        position += body_len;

        if position + 2 > self.response_buffer.len() {
            ::log::error!("Response buffer is too short for response");

            return Err(anyhow::anyhow!("Response buffer is too short for response"));
        }

        (&mut self.response_buffer[position..position + 2]).copy_from_slice(b"\r\n");

        position += 2;

        let content_len = body_len + 2;

        // Clear content-len header value

        {
            let start = RESPONSE_HEADER_A.len();
            let end = start + RESPONSE_HEADER_B.len();

            (&mut self.response_buffer[start..end]).copy_from_slice(RESPONSE_HEADER_B);
        }

        // Set content-len header value

        {
            let mut buf = ::itoa::Buffer::new();
            let content_len_bytes = buf.format(content_len).as_bytes();

            let start = RESPONSE_HEADER_A.len();
            let end = start + content_len_bytes.len();

            (&mut self.response_buffer[start..end]).copy_from_slice(content_len_bytes);
        }

        // Write buffer to stream

        self.stream.write(&self.response_buffer[..position]).await?;
        self.stream.flush().await?;

        Ok(())
    }
}

fn calculate_request_consumer_index(config: &Config, info_hash: InfoHash) -> usize {
    (info_hash.0[0] as usize) % config.request_workers
}

fn create_tcp_listener(config: &Config) -> TcpListener {
    let domain = if config.network.address.is_ipv4() {
        socket2::Domain::IPV4
    } else {
        socket2::Domain::IPV6
    };

    let socket = socket2::Socket::new(domain, socket2::Type::STREAM, Some(socket2::Protocol::TCP))
        .expect("create socket");

    if config.network.only_ipv6 {
        socket.set_only_v6(true).expect("socket: set only ipv6");
    }

    socket.set_reuse_port(true).expect("socket: set reuse port");

    socket
        .bind(&config.network.address.into())
        .unwrap_or_else(|err| panic!("socket: bind to {}: {:?}", config.network.address, err));

    socket
        .listen(config.network.tcp_backlog)
        .unwrap_or_else(|err| panic!("socket: listen {}: {:?}", config.network.address, err));

    unsafe { TcpListener::from_raw_fd(socket.into_raw_fd()) }
}
