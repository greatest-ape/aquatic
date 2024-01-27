use std::borrow::Cow;
use std::cell::RefCell;
use std::collections::BTreeMap;
use std::rc::Rc;
use std::sync::Arc;
use std::time::Duration;

use anyhow::Context;
use aquatic_common::access_list::{create_access_list_cache, AccessListArcSwap, AccessListCache};
use aquatic_common::rustls_config::RustlsConfig;
use aquatic_common::ServerStartInstant;
use aquatic_ws_protocol::common::{InfoHash, PeerId, ScrapeAction};
use aquatic_ws_protocol::incoming::{
    AnnounceEvent, AnnounceRequest, InMessage, ScrapeRequest, ScrapeRequestInfoHashes,
};
use aquatic_ws_protocol::outgoing::{
    ErrorResponse, ErrorResponseAction, OutMessage, ScrapeResponse, ScrapeStatistics,
};
use arc_swap::ArcSwap;
use async_tungstenite::WebSocketStream;
use futures::stream::{SplitSink, SplitStream};
use futures::{AsyncWriteExt, StreamExt};
use futures_lite::future::race;
use futures_rustls::TlsAcceptor;
use glommio::channels::channel_mesh::Senders;
use glommio::channels::local_channel::{LocalReceiver, LocalSender};
use glommio::net::TcpStream;
use glommio::timer::timeout;
use glommio::{enclose, prelude::*};
use hashbrown::hash_map::Entry;
use hashbrown::HashMap;
use slab::Slab;

#[cfg(feature = "metrics")]
use metrics::{Counter, Gauge};

use crate::common::*;
use crate::config::Config;
use crate::workers::socket::calculate_in_message_consumer_index;

#[cfg(feature = "metrics")]
use crate::workers::socket::{ip_version_to_metrics_str, WORKER_INDEX};

/// Optional second tuple field is for peer id hex representation
#[cfg(feature = "metrics")]
type PeerClientGauge = (Gauge, Option<Gauge>);

pub struct ConnectionRunner {
    pub config: Rc<Config>,
    pub access_list: Arc<AccessListArcSwap>,
    pub in_message_senders: Rc<Senders<(InMessageMeta, InMessage)>>,
    pub connection_valid_until: Rc<RefCell<ValidUntil>>,
    pub out_message_sender: Rc<LocalSender<(OutMessageMeta, OutMessage)>>,
    pub out_message_receiver: LocalReceiver<(OutMessageMeta, OutMessage)>,
    pub server_start_instant: ServerStartInstant,
    pub out_message_consumer_id: ConsumerId,
    pub connection_id: ConnectionId,
    pub opt_tls_config: Option<Arc<ArcSwap<RustlsConfig>>>,
    pub ip_version: IpVersion,
}

impl ConnectionRunner {
    pub async fn run(
        self,
        control_message_senders: Rc<Senders<SwarmControlMessage>>,
        close_conn_receiver: LocalReceiver<()>,
        stream: TcpStream,
    ) {
        let clean_up_data = ConnectionCleanupData {
            announced_info_hashes: Default::default(),
            ip_version: self.ip_version,
            #[cfg(feature = "metrics")]
            opt_peer_client: Default::default(),
            #[cfg(feature = "metrics")]
            active_connections_gauge: ::metrics::gauge!(
                "aquatic_active_connections",
                "ip_version" => ip_version_to_metrics_str(self.ip_version),
                "worker_index" => WORKER_INDEX.get().to_string(),
            ),
        };

        clean_up_data.before_open();

        let config = self.config.clone();
        let connection_id = self.connection_id;

        race(
            async {
                if let Err(err) = self.run_inner(clean_up_data.clone(), stream).await {
                    ::log::debug!("connection {:?} closed: {:#}", connection_id, err);
                }
            },
            async {
                close_conn_receiver.recv().await;
            },
        )
        .await;

        ::log::debug!("connection {:?} starting clean up", connection_id);

        clean_up_data
            .after_close(&config, control_message_senders)
            .await;

        ::log::debug!("connection {:?} finished clean up", connection_id);
    }

    async fn run_inner(
        self,
        clean_up_data: ConnectionCleanupData,
        mut stream: TcpStream,
    ) -> anyhow::Result<()> {
        if let Some(tls_config) = self.opt_tls_config.as_ref() {
            let tls_config = tls_config.load_full();
            let tls_acceptor = TlsAcceptor::from(tls_config);

            let stream = tls_acceptor.accept(stream).await?;

            self.run_inner_stream_agnostic(clean_up_data, stream).await
        } else {
            // Implementing this over TLS is too cumbersome, since the crate used
            // for TLS streams doesn't support peek and tungstenite doesn't
            // properly support sending a HTTP error response in accept_hdr
            // callback.
            if self.config.network.enable_http_health_checks {
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

            self.run_inner_stream_agnostic(clean_up_data, stream).await
        }
    }

    async fn run_inner_stream_agnostic<S>(
        self,
        clean_up_data: ConnectionCleanupData,
        stream: S,
    ) -> anyhow::Result<()>
    where
        S: futures::AsyncRead + futures::AsyncWrite + Unpin + 'static,
    {
        let ws_config = tungstenite::protocol::WebSocketConfig {
            max_frame_size: Some(self.config.network.websocket_max_frame_size),
            max_message_size: Some(self.config.network.websocket_max_message_size),
            write_buffer_size: self.config.network.websocket_write_buffer_size,
            max_write_buffer_size: self.config.network.websocket_write_buffer_size * 3,
            ..Default::default()
        };
        let stream = async_tungstenite::accept_async_with_config(stream, Some(ws_config)).await?;
        let (ws_out, ws_in) = futures::StreamExt::split(stream);

        let pending_scrape_slab = Rc::new(RefCell::new(Slab::new()));
        let access_list_cache = create_access_list_cache(&self.access_list);

        let config = self.config.clone();

        let reader_future = enclose!((pending_scrape_slab, clean_up_data) async move {
            let mut reader = ConnectionReader {
                config: self.config.clone(),
                access_list_cache,
                in_message_senders: self.in_message_senders,
                out_message_sender: self.out_message_sender,
                pending_scrape_slab,
                out_message_consumer_id: self.out_message_consumer_id,
                ws_in,
                ip_version: self.ip_version,
                connection_id: self.connection_id,
                clean_up_data: clean_up_data.clone(),
                #[cfg(feature = "metrics")]
                total_announce_requests_counter: ::metrics::counter!(
                    "aquatic_requests_total",
                    "type" => "announce",
                    "ip_version" => ip_version_to_metrics_str(self.ip_version),
                    "worker_index" => WORKER_INDEX.with(|index| index.get()).to_string(),
                ),
                #[cfg(feature = "metrics")]
                total_scrape_requests_counter: ::metrics::counter!(
                    "aquatic_requests_total",
                    "type" => "scrape",
                    "ip_version" => ip_version_to_metrics_str(self.ip_version),
                    "worker_index" => WORKER_INDEX.with(|index| index.get()).to_string(),
                )
            };

            reader.run_in_message_loop().await
        });

        let writer_future = async move {
            let mut writer = ConnectionWriter {
                config,
                out_message_receiver: self.out_message_receiver,
                connection_valid_until: self.connection_valid_until,
                ws_out,
                pending_scrape_slab,
                server_start_instant: self.server_start_instant,
                ip_version: self.ip_version,
                clean_up_data,
            };

            writer.run_out_message_loop().await
        };

        race(reader_future, writer_future).await
    }
}

struct ConnectionReader<S> {
    config: Rc<Config>,
    access_list_cache: AccessListCache,
    in_message_senders: Rc<Senders<(InMessageMeta, InMessage)>>,
    out_message_sender: Rc<LocalSender<(OutMessageMeta, OutMessage)>>,
    pending_scrape_slab: Rc<RefCell<Slab<PendingScrapeResponse>>>,
    out_message_consumer_id: ConsumerId,
    ws_in: SplitStream<WebSocketStream<S>>,
    ip_version: IpVersion,
    connection_id: ConnectionId,
    clean_up_data: ConnectionCleanupData,
    #[cfg(feature = "metrics")]
    total_announce_requests_counter: Counter,
    #[cfg(feature = "metrics")]
    total_scrape_requests_counter: Counter,
}

impl<S: futures::AsyncRead + futures::AsyncWrite + Unpin> ConnectionReader<S> {
    async fn run_in_message_loop(&mut self) -> anyhow::Result<()> {
        loop {
            let message = self
                .ws_in
                .next()
                .await
                .ok_or_else(|| anyhow::anyhow!("Stream ended"))??;

            match &message {
                tungstenite::Message::Text(_) | tungstenite::Message::Binary(_) => {
                    match InMessage::from_ws_message(message) {
                        Ok(InMessage::AnnounceRequest(request)) => {
                            self.handle_announce_request(request).await?;
                        }
                        Ok(InMessage::ScrapeRequest(request)) => {
                            self.handle_scrape_request(request).await?;
                        }
                        Err(err) => {
                            ::log::debug!("Couldn't parse in_message: {:#}", err);

                            self.send_error_response("Invalid request".into(), None, None)
                                .await?;
                        }
                    }
                }
                tungstenite::Message::Ping(_) => {
                    ::log::trace!("Received ping message");
                    // tungstenite sends a pong response by itself
                }
                tungstenite::Message::Pong(_) => {
                    ::log::trace!("Received pong message");
                }
                tungstenite::Message::Close(_) => {
                    ::log::debug!("Client sent close frame");

                    break Ok(());
                }
                tungstenite::Message::Frame(_) => {
                    ::log::warn!("Read raw websocket frame, this should not happen");
                }
            }

            yield_if_needed().await;
        }
    }

    // Silence RefCell lint due to false positives
    #[allow(clippy::await_holding_refcell_ref)]
    async fn handle_announce_request(&mut self, request: AnnounceRequest) -> anyhow::Result<()> {
        #[cfg(feature = "metrics")]
        self.total_announce_requests_counter.increment(1);

        let info_hash = request.info_hash;

        if self
            .access_list_cache
            .load()
            .allows(self.config.access_list.mode, &info_hash.0)
        {
            let mut announced_info_hashes = self.clean_up_data.announced_info_hashes.borrow_mut();

            // Store peer id / check if stored peer id matches
            match announced_info_hashes.entry(request.info_hash) {
                Entry::Occupied(entry) => {
                    if *entry.get() != request.peer_id {
                        // Drop Rc borrow before awaiting
                        drop(announced_info_hashes);

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
                    entry.insert(request.peer_id);

                    // Set peer client info if not set
                    #[cfg(feature = "metrics")]
                    if self.config.metrics.run_prometheus_endpoint
                        && self.config.metrics.peer_clients
                        && self.clean_up_data.opt_peer_client.borrow().is_none()
                    {
                        let peer_id = aquatic_peer_id::PeerId(request.peer_id.0);

                        let peer_client_gauge = ::metrics::gauge!(
                            "aquatic_peer_clients",
                            "client" => peer_id.client().to_string(),
                        );

                        peer_client_gauge.increment(1.0);

                        let opt_peer_id_prefix_gauge =
                            self.config.metrics.peer_id_prefixes.then(|| {
                                let g = ::metrics::gauge!(
                                    "aquatic_peer_id_prefixes",
                                    "prefix_hex" => peer_id.first_8_bytes_hex().to_string(),
                                );

                                g.increment(1.0);

                                g
                            });

                        *self.clean_up_data.opt_peer_client.borrow_mut() =
                            Some((peer_client_gauge, opt_peer_id_prefix_gauge));
                    };
                }
            }

            if let Some(AnnounceEvent::Stopped) = request.event {
                announced_info_hashes.remove(&request.info_hash);
            }

            // Drop Rc borrow before awaiting
            drop(announced_info_hashes);

            let in_message = InMessage::AnnounceRequest(request);

            let consumer_index = calculate_in_message_consumer_index(&self.config, info_hash);

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

        Ok(())
    }

    async fn handle_scrape_request(&mut self, request: ScrapeRequest) -> anyhow::Result<()> {
        #[cfg(feature = "metrics")]
        self.total_scrape_requests_counter.increment(1);

        let info_hashes = if let Some(info_hashes) = request.info_hashes {
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
                action: ScrapeAction::Scrape,
                info_hashes: Some(ScrapeRequestInfoHashes::Multiple(info_hashes)),
            });

            // Only fails when receiver is closed
            self.in_message_senders
                .send_to(consumer_index, (meta, in_message))
                .await
                .unwrap();
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
            .map_err(|err| {
                anyhow::anyhow!("ConnectionReader::send_error_response failed: {:#}", err)
            })
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
    connection_valid_until: Rc<RefCell<ValidUntil>>,
    ws_out: SplitSink<WebSocketStream<S>, tungstenite::Message>,
    pending_scrape_slab: Rc<RefCell<Slab<PendingScrapeResponse>>>,
    server_start_instant: ServerStartInstant,
    ip_version: IpVersion,
    clean_up_data: ConnectionCleanupData,
}

impl<S: futures::AsyncRead + futures::AsyncWrite + Unpin> ConnectionWriter<S> {
    // Silence RefCell lint due to false positives
    #[allow(clippy::await_holding_refcell_ref)]
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

                    let mut pending_responses = self.pending_scrape_slab.borrow_mut();

                    let pending_response = pending_responses
                        .get_mut(pending_scrape_id.0 as usize)
                        .ok_or(anyhow::anyhow!("pending scrape not found in slab"))?;

                    pending_response.stats.extend(out_message.files);
                    pending_response.pending_worker_out_messages -= 1;

                    if pending_response.pending_worker_out_messages == 0 {
                        let pending_response =
                            pending_responses.remove(pending_scrape_id.0 as usize);

                        pending_responses.shrink_to_fit();

                        let out_message = OutMessage::ScrapeResponse(ScrapeResponse {
                            action: ScrapeAction::Scrape,
                            files: pending_response.stats,
                        });

                        // Drop Rc borrow before awaiting
                        drop(pending_responses);

                        self.send_out_message(&out_message).await?;
                    }
                }
                out_message => {
                    self.send_out_message(&out_message).await?;
                }
            };

            yield_if_needed().await;
        }
    }

    async fn send_out_message(&mut self, out_message: &OutMessage) -> anyhow::Result<()> {
        timeout(Duration::from_secs(10), async {
            Ok(futures::SinkExt::send(&mut self.ws_out, out_message.to_ws_message()).await)
        })
        .await
        .map_err(|err| {
            anyhow::anyhow!("send_out_message: sending to peer took too long: {:#}", err)
        })?
        .with_context(|| "send_out_message")?;

        if let OutMessage::AnnounceResponse(_) | OutMessage::ScrapeResponse(_) = out_message {
            *self.connection_valid_until.borrow_mut() = ValidUntil::new(
                self.server_start_instant,
                self.config.cleaning.max_connection_idle,
            );
        }

        #[cfg(feature = "metrics")]
        {
            let out_message_type = match &out_message {
                OutMessage::OfferOutMessage(_) => "offer",
                OutMessage::AnswerOutMessage(_) => "offer_answer",
                OutMessage::AnnounceResponse(_) => "announce",
                OutMessage::ScrapeResponse(_) => "scrape",
                OutMessage::ErrorResponse(_) => "error",
            };

            ::metrics::counter!(
                "aquatic_responses_total",
                "type" => out_message_type,
                "ip_version" => ip_version_to_metrics_str(self.ip_version),
                "worker_index" => WORKER_INDEX.with(|index| index.get()).to_string(),
            )
            .increment(1);

            // As long as connection is still alive, increment peer client
            // gauges by zero to prevent them from being removed due to
            // idleness
            if let Some((peer_client_gauge, opt_peer_id_prefix_gauge)) =
                self.clean_up_data.opt_peer_client.borrow().as_ref()
            {
                peer_client_gauge.increment(0.0);

                if let Some(g) = opt_peer_id_prefix_gauge {
                    g.increment(0.0);
                }
            }
        }

        Ok(())
    }
}

/// Data stored with connection needed for cleanup after it closes
#[derive(Clone)]
struct ConnectionCleanupData {
    announced_info_hashes: Rc<RefCell<HashMap<InfoHash, PeerId>>>,
    ip_version: IpVersion,
    #[cfg(feature = "metrics")]
    opt_peer_client: Rc<RefCell<Option<PeerClientGauge>>>,
    #[cfg(feature = "metrics")]
    active_connections_gauge: Gauge,
}

impl ConnectionCleanupData {
    fn before_open(&self) {
        #[cfg(feature = "metrics")]
        self.active_connections_gauge.increment(1.0);
    }
    async fn after_close(
        &self,
        config: &Config,
        control_message_senders: Rc<Senders<SwarmControlMessage>>,
    ) {
        let mut announced_info_hashes = HashMap::new();

        for (info_hash, peer_id) in self.announced_info_hashes.take().into_iter() {
            let consumer_index = calculate_in_message_consumer_index(config, info_hash);

            announced_info_hashes
                .entry(consumer_index)
                .or_insert(Vec::new())
                .push((info_hash, peer_id));
        }

        for (consumer_index, announced_info_hashes) in announced_info_hashes.into_iter() {
            let message = SwarmControlMessage::ConnectionClosed {
                ip_version: self.ip_version,
                announced_info_hashes,
            };

            control_message_senders
                .send_to(consumer_index, message)
                .await
                .expect("control message receiver open");
        }

        #[cfg(feature = "metrics")]
        self.active_connections_gauge.decrement(1.0);

        #[cfg(feature = "metrics")]
        if let Some((peer_client_gauge, opt_peer_id_prefix_gauge)) = self.opt_peer_client.take() {
            peer_client_gauge.decrement(1.0);

            if let Some(g) = opt_peer_id_prefix_gauge {
                g.decrement(1.0);
            }
        }
    }
}

struct PendingScrapeResponse {
    pending_worker_out_messages: usize,
    stats: HashMap<InfoHash, ScrapeStatistics>,
}
