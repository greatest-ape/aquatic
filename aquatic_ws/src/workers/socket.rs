use std::borrow::Cow;
use std::cell::RefCell;
use std::collections::BTreeMap;
use std::os::unix::prelude::{FromRawFd, IntoRawFd};
use std::rc::Rc;
use std::sync::Arc;
use std::time::Duration;

use anyhow::Context;
use aquatic_common::access_list::{create_access_list_cache, AccessListArcSwap, AccessListCache};
use aquatic_common::privileges::PrivilegeDropper;
use aquatic_common::rustls_config::RustlsConfig;
use aquatic_common::{PanicSentinel, ServerStartInstant};
use aquatic_ws_protocol::*;
use async_tungstenite::WebSocketStream;
use futures::stream::{SplitSink, SplitStream};
use futures::{AsyncWriteExt, StreamExt};
use futures_lite::future::race;
use futures_rustls::TlsAcceptor;
use glommio::channels::channel_mesh::{MeshBuilder, Partial, Role, Senders};
use glommio::channels::local_channel::{new_bounded, LocalReceiver, LocalSender};
use glommio::channels::shared_channel::ConnectedReceiver;
use glommio::net::{TcpListener, TcpStream};
use glommio::task::JoinHandle;
use glommio::timer::{sleep, timeout, TimerActionRepeat};
use glommio::{enclose, prelude::*};
use hashbrown::hash_map::Entry;
use hashbrown::HashMap;
use slab::Slab;

use crate::config::Config;

use crate::common::*;

const LOCAL_CHANNEL_SIZE: usize = 16;

struct PendingScrapeResponse {
    pending_worker_out_messages: usize,
    stats: HashMap<InfoHash, ScrapeStatistics>,
}

struct ConnectionReference {
    task_handle: Option<JoinHandle<()>>,
    /// Sender part of channel used to pass on outgoing messages from request
    /// worker
    out_message_sender: Rc<LocalSender<(OutMessageMeta, OutMessage)>>,
    /// Updated after sending message to peer
    valid_until: ValidUntil,
    announced_info_hashes: HashMap<InfoHash, PeerId>,
    ip_version: IpVersion,
}

pub async fn run_socket_worker(
    _sentinel: PanicSentinel,
    config: Config,
    state: State,
    opt_tls_config: Option<Arc<RustlsConfig>>,
    control_message_mesh_builder: MeshBuilder<SwarmControlMessage, Partial>,
    in_message_mesh_builder: MeshBuilder<(InMessageMeta, InMessage), Partial>,
    out_message_mesh_builder: MeshBuilder<(OutMessageMeta, OutMessage), Partial>,
    priv_dropper: PrivilegeDropper,
    server_start_instant: ServerStartInstant,
) {
    let config = Rc::new(config);
    let access_list = state.access_list;

    let listener = create_tcp_listener(&config, priv_dropper).expect("create tcp listener");

    ::log::info!("created tcp listener");

    let (control_message_senders, _) = control_message_mesh_builder
        .join(Role::Producer)
        .await
        .unwrap();
    let control_message_senders = Rc::new(control_message_senders);

    let (in_message_senders, _) = in_message_mesh_builder.join(Role::Producer).await.unwrap();
    let in_message_senders = Rc::new(in_message_senders);

    let tq_prioritized = executor().create_task_queue(
        Shares::Static(100),
        Latency::Matters(Duration::from_millis(1)),
        "prioritized",
    );
    let tq_regular =
        executor().create_task_queue(Shares::Static(1), Latency::NotImportant, "regular");

    let (_, mut out_message_receivers) =
        out_message_mesh_builder.join(Role::Consumer).await.unwrap();
    let out_message_consumer_id = ConsumerId(
        out_message_receivers
            .consumer_id()
            .unwrap()
            .try_into()
            .unwrap(),
    );

    ::log::info!("joined channels");

    let connection_slab = Rc::new(RefCell::new(Slab::new()));

    // Periodically clean connections
    TimerActionRepeat::repeat_into(
        enclose!((config, connection_slab) move || {
            clean_connections(
                config.clone(),
                connection_slab.clone(),
                server_start_instant,
            )
        }),
        tq_prioritized,
    )
    .unwrap();

    for (_, out_message_receiver) in out_message_receivers.streams() {
        spawn_local_into(
            receive_out_messages(out_message_receiver, connection_slab.clone()),
            tq_regular,
        )
        .unwrap()
        .detach();
    }

    let mut incoming = listener.incoming();

    while let Some(stream) = incoming.next().await {
        match stream {
            Ok(stream) => {
                let ip_version = match stream.peer_addr() {
                    Ok(addr) => IpVersion::canonical_from_ip(addr.ip()),
                    Err(err) => {
                        ::log::info!("could not extract ip version (v4 or v6): {:#}", err);

                        continue;
                    }
                };

                let (out_message_sender, out_message_receiver) = new_bounded(LOCAL_CHANNEL_SIZE);
                let out_message_sender = Rc::new(out_message_sender);

                let key = RefCell::borrow_mut(&connection_slab).insert(ConnectionReference {
                    task_handle: None,
                    out_message_sender: out_message_sender.clone(),
                    valid_until: ValidUntil::new(
                        server_start_instant,
                        config.cleaning.max_connection_idle,
                    ),
                    announced_info_hashes: Default::default(),
                    ip_version,
                });

                ::log::trace!("accepting stream, assigning id {}", key);

                let task_handle = spawn_local_into(enclose!((config, access_list, control_message_senders, in_message_senders, connection_slab, opt_tls_config) async move {
                    if let Err(err) = run_connection(
                        config.clone(),
                        access_list,
                        in_message_senders,
                        tq_prioritized,
                        tq_regular,
                        connection_slab.clone(),
                        out_message_sender,
                        out_message_receiver,
                        server_start_instant,
                        out_message_consumer_id,
                        ConnectionId(key),
                        opt_tls_config,
                        ip_version,
                        stream,
                    ).await {
                        ::log::debug!("connection error: {:#}", err);
                    }

                    // Clean up after closed connection

                    // Remove reference in separate statement to avoid
                    // multiple RefCell borrows
                    let opt_reference = connection_slab.borrow_mut().try_remove(key);

                    // Tell swarm workers to remove peer
                    if let Some(reference) = opt_reference {
                        for (info_hash, peer_id) in reference.announced_info_hashes {
                            let message = SwarmControlMessage::ConnectionClosed {
                                info_hash,
                                peer_id,
                                ip_version: reference.ip_version,
                            };

                            let consumer_index =
                                calculate_in_message_consumer_index(&config, info_hash);

                            // Only fails when receiver is closed
                            control_message_senders
                                .send_to(
                                    consumer_index,
                                    message
                                )
                                .await
                                .unwrap();
                        }
                    }
                }), tq_regular)
                .unwrap()
                .detach();

                if let Some(reference) = connection_slab.borrow_mut().get_mut(key) {
                    reference.task_handle = Some(task_handle);
                }
            }
            Err(err) => {
                ::log::error!("accept connection: {:#}", err);
            }
        }
    }
}

async fn clean_connections(
    config: Rc<Config>,
    connection_slab: Rc<RefCell<Slab<ConnectionReference>>>,
    server_start_instant: ServerStartInstant,
) -> Option<Duration> {
    let now = server_start_instant.seconds_elapsed();

    connection_slab.borrow_mut().retain(|_, reference| {
        if reference.valid_until.valid(now) {
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

async fn receive_out_messages(
    mut out_message_receiver: ConnectedReceiver<(OutMessageMeta, OutMessage)>,
    connection_references: Rc<RefCell<Slab<ConnectionReference>>>,
) {
    let connection_references = &connection_references;

    while let Some((meta, out_message)) = out_message_receiver.next().await {
        if let Some(reference) = connection_references.borrow().get(meta.connection_id.0) {
            ::log::trace!(
                "local channel {} len: {}",
                meta.connection_id.0,
                reference.out_message_sender.len()
            );

            match reference.out_message_sender.try_send((meta, out_message)) {
                Ok(()) => {}
                Err(GlommioError::Closed(_)) => {}
                Err(GlommioError::WouldBlock(_)) => {}
                Err(err) => {
                    ::log::debug!(
                        "Couldn't send out_message from shared channel to local receiver: {:?}",
                        err
                    );
                }
            }
        }
    }
}

async fn run_connection(
    config: Rc<Config>,
    access_list: Arc<AccessListArcSwap>,
    in_message_senders: Rc<Senders<(InMessageMeta, InMessage)>>,
    tq_prioritized: TaskQueueHandle,
    tq_regular: TaskQueueHandle,
    connection_slab: Rc<RefCell<Slab<ConnectionReference>>>,
    out_message_sender: Rc<LocalSender<(OutMessageMeta, OutMessage)>>,
    out_message_receiver: LocalReceiver<(OutMessageMeta, OutMessage)>,
    server_start_instant: ServerStartInstant,
    out_message_consumer_id: ConsumerId,
    connection_id: ConnectionId,
    opt_tls_config: Option<Arc<RustlsConfig>>,
    ip_version: IpVersion,
    mut stream: TcpStream,
) -> anyhow::Result<()> {
    if let Some(tls_config) = opt_tls_config {
        let tls_acceptor: TlsAcceptor = tls_config.into();

        let stream = tls_acceptor.accept(stream).await?;

        run_stream_agnostic_connection(
            config.clone(),
            access_list,
            in_message_senders,
            tq_prioritized,
            tq_regular,
            connection_slab.clone(),
            out_message_sender,
            out_message_receiver,
            server_start_instant,
            out_message_consumer_id,
            connection_id,
            stream,
            ip_version,
        )
        .await
    } else {
        // Implementing this over TLS is too cumbersome, since the crate used
        // for TLS streams doesn't support peek and tungstenite doesn't
        // properly support sending a HTTP error response in accept_hdr
        // callback.
        if config.network.enable_http_health_checks {
            let mut peek_buf = [0u8; 11];

            stream
                .peek(&mut peek_buf)
                .await
                .map_err(|err| anyhow::anyhow!("error peeking: {:#}", err))?;

            if &peek_buf == b"GET /health" {
                stream
                    .write_all(b"HTTP/1.1 200 Ok\r\nContent-Length: 2\r\n\r\nOk")
                    .await
                    .map_err(|err| {
                        anyhow::anyhow!("error sending health check response: {:#}", err)
                    })?;
                stream.flush().await.map_err(|err| {
                    anyhow::anyhow!("error flushing health check response: {:#}", err)
                })?;

                return Err(anyhow::anyhow!(
                    "client requested health check, skipping websocket negotiation"
                ));
            }
        }

        run_stream_agnostic_connection(
            config.clone(),
            access_list,
            in_message_senders,
            tq_prioritized,
            tq_regular,
            connection_slab.clone(),
            out_message_sender,
            out_message_receiver,
            server_start_instant,
            out_message_consumer_id,
            connection_id,
            stream,
            ip_version,
        )
        .await
    }
}

async fn run_stream_agnostic_connection<
    S: futures::AsyncRead + futures::AsyncWrite + Unpin + 'static,
>(
    config: Rc<Config>,
    access_list: Arc<AccessListArcSwap>,
    in_message_senders: Rc<Senders<(InMessageMeta, InMessage)>>,
    tq_prioritized: TaskQueueHandle,
    tq_regular: TaskQueueHandle,
    connection_slab: Rc<RefCell<Slab<ConnectionReference>>>,
    out_message_sender: Rc<LocalSender<(OutMessageMeta, OutMessage)>>,
    out_message_receiver: LocalReceiver<(OutMessageMeta, OutMessage)>,
    server_start_instant: ServerStartInstant,
    out_message_consumer_id: ConsumerId,
    connection_id: ConnectionId,
    stream: S,
    ip_version: IpVersion,
) -> anyhow::Result<()> {
    let ws_config = tungstenite::protocol::WebSocketConfig {
        max_frame_size: Some(config.network.websocket_max_frame_size),
        max_message_size: Some(config.network.websocket_max_message_size),
        max_send_queue: Some(2),
        ..Default::default()
    };
    let stream = async_tungstenite::accept_async_with_config(stream, Some(ws_config)).await?;

    let (ws_out, ws_in) = futures::StreamExt::split(stream);

    let pending_scrape_slab = Rc::new(RefCell::new(Slab::new()));
    let access_list_cache = create_access_list_cache(&access_list);

    let reader_handle = spawn_local_into(
        enclose!((config, connection_slab, pending_scrape_slab) async move {
            let mut reader = ConnectionReader {
                config,
                access_list_cache,
                connection_slab,
                in_message_senders,
                out_message_sender,
                pending_scrape_slab,
                out_message_consumer_id,
                ws_in,
                ip_version,
                connection_id,
            };

            let result = reader.run_in_message_loop().await;

            result
        }),
        tq_regular,
    )
    .unwrap()
    .detach();

    let writer_handle = spawn_local_into(
        async move {
            let mut writer = ConnectionWriter {
                config,
                out_message_receiver,
                connection_slab,
                ws_out,
                pending_scrape_slab,
                connection_id,
                server_start_instant,
            };

            let result = writer.run_out_message_loop().await;

            result
        },
        tq_prioritized,
    )
    .unwrap()
    .detach();

    race(reader_handle, writer_handle)
        .await
        .expect("reader/writer task should not be closed")
}

struct ConnectionReader<S> {
    config: Rc<Config>,
    access_list_cache: AccessListCache,
    connection_slab: Rc<RefCell<Slab<ConnectionReference>>>,
    in_message_senders: Rc<Senders<(InMessageMeta, InMessage)>>,
    out_message_sender: Rc<LocalSender<(OutMessageMeta, OutMessage)>>,
    pending_scrape_slab: Rc<RefCell<Slab<PendingScrapeResponse>>>,
    out_message_consumer_id: ConsumerId,
    ws_in: SplitStream<WebSocketStream<S>>,
    ip_version: IpVersion,
    connection_id: ConnectionId,
}

impl<S: futures::AsyncRead + futures::AsyncWrite + Unpin> ConnectionReader<S> {
    async fn run_in_message_loop(&mut self) -> anyhow::Result<()> {
        loop {
            while self.out_message_sender.is_full() {
                sleep(Duration::from_millis(100)).await;

                yield_if_needed().await;
            }

            let message = self
                .ws_in
                .next()
                .await
                .ok_or_else(|| anyhow::anyhow!("Stream ended"))??;

            match InMessage::from_ws_message(message) {
                Ok(in_message) => {
                    self.handle_in_message(in_message).await?;
                }
                Err(err) => {
                    ::log::debug!("Couldn't parse in_message: {:?}", err);

                    self.send_error_response("Invalid request".into(), None, None)
                        .await?;
                }
            }

            yield_if_needed().await;
        }
    }

    async fn handle_in_message(&mut self, in_message: InMessage) -> anyhow::Result<()> {
        match in_message {
            InMessage::AnnounceRequest(announce_request) => {
                let info_hash = announce_request.info_hash;

                if self
                    .access_list_cache
                    .load()
                    .allows(self.config.access_list.mode, &info_hash.0)
                {
                    {
                        let mut connection_slab = self.connection_slab.borrow_mut();

                        let connection_reference = connection_slab
                            .get_mut(self.connection_id.0)
                            .ok_or_else(|| {
                                anyhow::anyhow!(
                                    "connection reference {} not found in slab",
                                    self.connection_id.0
                                )
                            })?;

                        // Store peer id / check if stored peer id matches
                        match connection_reference
                            .announced_info_hashes
                            .entry(announce_request.info_hash)
                        {
                            Entry::Occupied(entry) => {
                                if *entry.get() != announce_request.peer_id {
                                    // Drop Rc borrow before awaiting
                                    drop(connection_slab);

                                    self.send_error_response(
                                        "Only one peer id can be used per torrent".into(),
                                        Some(ErrorResponseAction::Announce),
                                        Some(info_hash),
                                    )
                                    .await?;

                                    return Err(anyhow::anyhow!(
                                        "Peer used more than one PeerId for a single torrent"
                                    ));
                                }
                            }
                            Entry::Vacant(entry) => {
                                entry.insert(announce_request.peer_id);
                            }
                        }
                    }

                    let in_message = InMessage::AnnounceRequest(announce_request);

                    let consumer_index =
                        calculate_in_message_consumer_index(&self.config, info_hash);

                    // Only fails when receiver is closed
                    self.in_message_senders
                        .send_to(
                            consumer_index,
                            (self.make_connection_meta(None), in_message),
                        )
                        .await
                        .unwrap();
                } else {
                    self.send_error_response(
                        "Info hash not allowed".into(),
                        Some(ErrorResponseAction::Announce),
                        Some(info_hash),
                    )
                    .await?;
                }
            }
            InMessage::ScrapeRequest(ScrapeRequest { info_hashes, .. }) => {
                let info_hashes = if let Some(info_hashes) = info_hashes {
                    info_hashes
                } else {
                    // If request.info_hashes is empty, don't return scrape for all
                    // torrents, even though reference server does it. It is too expensive.
                    self.send_error_response(
                        "Full scrapes are not allowed".into(),
                        Some(ErrorResponseAction::Scrape),
                        None,
                    )
                    .await?;

                    return Ok(());
                };

                let mut info_hashes_by_worker: BTreeMap<usize, Vec<InfoHash>> = BTreeMap::new();

                for info_hash in info_hashes.as_vec() {
                    let info_hashes = info_hashes_by_worker
                        .entry(calculate_in_message_consumer_index(&self.config, info_hash))
                        .or_default();

                    info_hashes.push(info_hash);
                }

                let pending_worker_out_messages = info_hashes_by_worker.len();

                let pending_scrape_response = PendingScrapeResponse {
                    pending_worker_out_messages,
                    stats: Default::default(),
                };

                let pending_scrape_id: u8 = self
                    .pending_scrape_slab
                    .borrow_mut()
                    .insert(pending_scrape_response)
                    .try_into()
                    .with_context(|| "Reached 256 pending scrape responses")?;

                let meta = self.make_connection_meta(Some(PendingScrapeId(pending_scrape_id)));

                for (consumer_index, info_hashes) in info_hashes_by_worker {
                    let in_message = InMessage::ScrapeRequest(ScrapeRequest {
                        action: ScrapeAction,
                        info_hashes: Some(ScrapeRequestInfoHashes::Multiple(info_hashes)),
                    });

                    // Only fails when receiver is closed
                    self.in_message_senders
                        .send_to(consumer_index, (meta, in_message))
                        .await
                        .unwrap();
                }
            }
        }

        Ok(())
    }

    async fn send_error_response(
        &self,
        failure_reason: Cow<'static, str>,
        action: Option<ErrorResponseAction>,
        info_hash: Option<InfoHash>,
    ) -> anyhow::Result<()> {
        let out_message = OutMessage::ErrorResponse(ErrorResponse {
            action,
            failure_reason,
            info_hash,
        });

        self.out_message_sender
            .send((self.make_connection_meta(None).into(), out_message))
            .await
            .map_err(|err| anyhow::anyhow!("ConnectionReader::send_error_response failed: {}", err))
    }

    fn make_connection_meta(&self, pending_scrape_id: Option<PendingScrapeId>) -> InMessageMeta {
        InMessageMeta {
            connection_id: self.connection_id,
            out_message_consumer_id: self.out_message_consumer_id,
            ip_version: self.ip_version,
            pending_scrape_id,
        }
    }
}

struct ConnectionWriter<S> {
    config: Rc<Config>,
    out_message_receiver: LocalReceiver<(OutMessageMeta, OutMessage)>,
    connection_slab: Rc<RefCell<Slab<ConnectionReference>>>,
    ws_out: SplitSink<WebSocketStream<S>, tungstenite::Message>,
    pending_scrape_slab: Rc<RefCell<Slab<PendingScrapeResponse>>>,
    server_start_instant: ServerStartInstant,
    connection_id: ConnectionId,
}

impl<S: futures::AsyncRead + futures::AsyncWrite + Unpin> ConnectionWriter<S> {
    async fn run_out_message_loop(&mut self) -> anyhow::Result<()> {
        loop {
            let (meta, out_message) = self.out_message_receiver.recv().await.ok_or_else(|| {
                anyhow::anyhow!("ConnectionWriter couldn't receive message, sender is closed")
            })?;

            match out_message {
                OutMessage::ScrapeResponse(out_message) => {
                    let pending_scrape_id = meta
                        .pending_scrape_id
                        .expect("meta.pending_scrape_id not set");

                    let finished = if let Some(pending) = Slab::get_mut(
                        &mut RefCell::borrow_mut(&self.pending_scrape_slab),
                        pending_scrape_id.0 as usize,
                    ) {
                        pending.stats.extend(out_message.files);
                        pending.pending_worker_out_messages -= 1;

                        pending.pending_worker_out_messages == 0
                    } else {
                        return Err(anyhow::anyhow!("pending scrape not found in slab"));
                    };

                    if finished {
                        let out_message = {
                            let mut slab = RefCell::borrow_mut(&self.pending_scrape_slab);

                            let pending = slab.remove(pending_scrape_id.0 as usize);

                            slab.shrink_to_fit();

                            OutMessage::ScrapeResponse(ScrapeResponse {
                                action: ScrapeAction,
                                files: pending.stats,
                            })
                        };

                        self.send_out_message(&out_message).await?;
                    }
                }
                out_message => {
                    self.send_out_message(&out_message).await?;
                }
            };
        }
    }

    async fn send_out_message(&mut self, out_message: &OutMessage) -> anyhow::Result<()> {
        let result = timeout(Duration::from_secs(10), async {
            let result =
                futures::SinkExt::send(&mut self.ws_out, out_message.to_ws_message()).await;

            Ok(result)
        })
        .await;

        match result {
            Ok(Ok(())) => {
                self.connection_slab
                    .borrow_mut()
                    .get_mut(self.connection_id.0)
                    .ok_or_else(|| {
                        anyhow::anyhow!(
                            "connection reference {} not found in slab",
                            self.connection_id.0
                        )
                    })?
                    .valid_until = ValidUntil::new(
                    self.server_start_instant,
                    self.config.cleaning.max_connection_idle,
                );

                Ok(())
            }
            Ok(Err(err)) => Err(err.into()),
            Err(err) => {
                ::log::debug!("send_out_message: sending to peer took to long: {}", err);

                Ok(())
            }
        }
    }
}

fn calculate_in_message_consumer_index(config: &Config, info_hash: InfoHash) -> usize {
    (info_hash.0[0] as usize) % config.swarm_workers
}

fn create_tcp_listener(
    config: &Config,
    priv_dropper: PrivilegeDropper,
) -> anyhow::Result<TcpListener> {
    let domain = if config.network.address.is_ipv4() {
        socket2::Domain::IPV4
    } else {
        socket2::Domain::IPV6
    };

    ::log::info!("creating socket..");

    let socket = socket2::Socket::new(domain, socket2::Type::STREAM, Some(socket2::Protocol::TCP))
        .with_context(|| "create socket")?;

    if config.network.only_ipv6 {
        ::log::info!("setting socket to ipv6 only..");

        socket
            .set_only_v6(true)
            .with_context(|| "socket: set only ipv6")?;
    }

    ::log::info!("setting SO_REUSEPORT on socket..");

    socket
        .set_reuse_port(true)
        .with_context(|| "socket: set reuse port")?;

    ::log::info!("binding socket..");

    socket
        .bind(&config.network.address.into())
        .with_context(|| format!("socket: bind to {}", config.network.address))?;

    ::log::info!("listening on socket..");

    socket
        .listen(config.network.tcp_backlog)
        .with_context(|| format!("socket: listen {}", config.network.address))?;

    ::log::info!("running PrivilegeDropper::after_socket_creation..");

    priv_dropper.after_socket_creation()?;

    ::log::info!("casting socket to glommio TcpListener..");

    Ok(unsafe { TcpListener::from_raw_fd(socket.into_raw_fd()) })
}
