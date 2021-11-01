use std::cell::RefCell;
use std::collections::BTreeMap;
use std::net::SocketAddr;
use std::rc::Rc;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;
use std::time::Duration;

use aquatic_common::access_list::AccessList;
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
use glommio::timer::TimerActionRepeat;
use glommio::{enclose, prelude::*};
use slab::Slab;

use crate::common::num_digits_in_usize;
use crate::config::Config;

use super::common::*;

const INTERMEDIATE_BUFFER_SIZE: usize = 1024;
const MAX_REQUEST_SIZE: usize = 2048;

struct PendingScrapeResponse {
    pending_worker_responses: usize,
    stats: BTreeMap<InfoHash, ScrapeStatistics>,
}

struct ConnectionReference {
    response_sender: LocalSender<ChannelResponse>,
}

struct Connection {
    config: Rc<Config>,
    access_list: Rc<RefCell<AccessList>>,
    request_senders: Rc<Senders<ChannelRequest>>,
    response_receiver: LocalReceiver<ChannelResponse>,
    response_consumer_id: ConsumerId,
    stream: TlsStream<TcpStream>,
    peer_addr: SocketAddr,
    connection_id: ConnectionId,
    request_buffer: [u8; MAX_REQUEST_SIZE],
    request_buffer_position: usize,
}

pub async fn run_socket_worker(
    config: Config,
    tls_config: Arc<TlsConfig>,
    request_mesh_builder: MeshBuilder<ChannelRequest, Partial>,
    response_mesh_builder: MeshBuilder<ChannelResponse, Partial>,
    num_bound_sockets: Arc<AtomicUsize>,
    access_list: AccessList,
) {
    let config = Rc::new(config);
    let access_list = Rc::new(RefCell::new(access_list));

    let listener = TcpListener::bind(config.network.address).expect("bind socket");
    num_bound_sockets.fetch_add(1, Ordering::SeqCst);

    let (request_senders, _) = request_mesh_builder.join(Role::Producer).await.unwrap();
    let request_senders = Rc::new(request_senders);

    let (_, mut response_receivers) = response_mesh_builder.join(Role::Consumer).await.unwrap();
    let response_consumer_id = ConsumerId(response_receivers.consumer_id().unwrap());

    let connection_slab = Rc::new(RefCell::new(Slab::new()));
    let connections_to_remove = Rc::new(RefCell::new(Vec::new()));

    // Periodically update access list
    TimerActionRepeat::repeat(enclose!((config, access_list) move || {
        enclose!((config, access_list) move || async move {
            update_access_list(config.clone(), access_list.clone()).await;

            Some(Duration::from_secs(config.cleaning.interval))
        })()
    }));

    // Periodically remove closed connections
    TimerActionRepeat::repeat(
        enclose!((config, connection_slab, connections_to_remove) move || {
            remove_closed_connections(
                config.clone(),
                connection_slab.clone(),
                connections_to_remove.clone(),
            )
        }),
    );

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
                let key = connection_slab
                    .borrow_mut()
                    .insert(ConnectionReference { response_sender });

                spawn_local(enclose!((config, access_list, request_senders, tls_config, connections_to_remove) async move {
                    if let Err(err) = Connection::run(
                        config,
                        access_list,
                        request_senders,
                        response_receiver,
                        response_consumer_id,
                        ConnectionId(key),
                        tls_config,
                        stream
                    ).await {
                        ::log::info!("Connection::run() error: {:?}", err);
                    }

                    connections_to_remove.borrow_mut().push(key);
                }))
                .detach();
            }
            Err(err) => {
                ::log::error!("accept connection: {:?}", err);
            }
        }
    }
}

async fn remove_closed_connections(
    config: Rc<Config>,
    connection_slab: Rc<RefCell<Slab<ConnectionReference>>>,
    connections_to_remove: Rc<RefCell<Vec<usize>>>,
) -> Option<Duration> {
    let connections_to_remove = connections_to_remove.replace(Vec::new());

    for connection_id in connections_to_remove {
        if let Some(_) = connection_slab.borrow_mut().try_remove(connection_id) {
            ::log::debug!("removed connection with id {}", connection_id);
        } else {
            ::log::error!(
                "couldn't remove connection with id {}, it is not in connection slab",
                connection_id
            );
        }
    }

    Some(Duration::from_secs(config.cleaning.interval))
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

impl Connection {
    async fn run(
        config: Rc<Config>,
        access_list: Rc<RefCell<AccessList>>,
        request_senders: Rc<Senders<ChannelRequest>>,
        response_receiver: LocalReceiver<ChannelResponse>,
        response_consumer_id: ConsumerId,
        connection_id: ConnectionId,
        tls_config: Arc<TlsConfig>,
        stream: TcpStream,
    ) -> anyhow::Result<()> {
        let peer_addr = stream
            .peer_addr()
            .map_err(|err| anyhow::anyhow!("Couldn't get peer addr: {:?}", err))?;

        let tls_acceptor: TlsAcceptor = tls_config.into();
        let stream = tls_acceptor.accept(stream).await?;

        let mut conn = Connection {
            config: config.clone(),
            access_list: access_list.clone(),
            request_senders: request_senders.clone(),
            response_receiver,
            response_consumer_id,
            stream,
            peer_addr,
            connection_id,
            request_buffer: [0u8; MAX_REQUEST_SIZE],
            request_buffer_position: 0,
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
        let mut buf = [0u8; INTERMEDIATE_BUFFER_SIZE];

        loop {
            ::log::debug!("read");

            let bytes_read = self.stream.read(&mut buf).await?;
            let request_buffer_end = self.request_buffer_position + bytes_read;

            if request_buffer_end > self.request_buffer.len() {
                return Err(anyhow::anyhow!("request too large"));
            } else {
                let request_buffer_slice =
                    &mut self.request_buffer[self.request_buffer_position..request_buffer_end];

                request_buffer_slice.copy_from_slice(&buf[..bytes_read]);

                self.request_buffer_position = request_buffer_end;

                match Request::from_bytes(&self.request_buffer[..self.request_buffer_position]) {
                    Ok(request) => {
                        ::log::debug!("received request: {:?}", request);

                        self.request_buffer_position = 0;

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
    }

    /// Take a request and:
    /// - Return error response if request is not allowed
    /// - If it is an announce request, send it to request workers an await a
    ///   response
    /// - If it is a scrape requests, split it up, pass on the parts to
    ///   relevant request workers and await a response
    async fn handle_request(&self, request: Request) -> anyhow::Result<Response> {
        match request {
            Request::Announce(request) => {
                let info_hash = request.info_hash;

                if self
                    .access_list
                    .borrow()
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
            if let Some(channel_response) = self.response_receiver.recv().await {
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
            } else {
                // TODO: this is a serious error condition and should maybe be handled differently
                return Err(anyhow::anyhow!(
                    "response receiver can't receive - sender is closed"
                ));
            }
        }
    }

    async fn write_response(&mut self, response: &Response) -> anyhow::Result<()> {
        let mut body = Vec::new();

        response.write(&mut body).unwrap();

        let content_len = body.len() + 2; // 2 is for newlines at end
        let content_len_num_digits = num_digits_in_usize(content_len);

        let mut response_bytes = Vec::with_capacity(39 + content_len_num_digits + body.len());

        response_bytes.extend_from_slice(b"HTTP/1.1 200 OK\r\nContent-Length: ");
        ::itoa::write(&mut response_bytes, content_len)?;
        response_bytes.extend_from_slice(b"\r\n\r\n");
        response_bytes.append(&mut body);
        response_bytes.extend_from_slice(b"\r\n");

        self.stream.write(&response_bytes[..]).await?;
        self.stream.flush().await?;

        Ok(())
    }
}

fn calculate_request_consumer_index(config: &Config, info_hash: InfoHash) -> usize {
    (info_hash.0[0] as usize) % config.request_workers
}
