use std::io::ErrorKind;
use std::sync::Arc;
use std::time::{Duration, Instant};
use std::vec::Drain;

use anyhow::Context;
use aquatic_common::access_list::AccessListQuery;
use crossbeam_channel::Receiver;
use hashbrown::HashMap;
use mio::net::TcpListener;
use mio::{Events, Interest, Poll, Token};
use socket2::{Domain, Protocol, Socket, Type};
use tungstenite::protocol::WebSocketConfig;

use aquatic_common::convert_ipv4_mapped_ipv6;
use aquatic_ws_protocol::*;

use crate::common::*;
use crate::config::Config;

pub mod connection;

use super::common::*;

use connection::{Connection, NotRegistered, Registered};

struct ConnectionMap {
    token_counter: Token,
    connections: HashMap<Token, Connection<Registered>>,
}

impl Default for ConnectionMap {
    fn default() -> Self {
        Self {
            token_counter: Token(2),
            connections: Default::default(),
        }
    }
}

impl ConnectionMap {
    fn insert_and_register_new<F>(&mut self, poll: &mut Poll, connection_creator: F)
    where
        F: FnOnce(Token) -> Connection<NotRegistered>,
    {
        self.token_counter.0 = self.token_counter.0.wrapping_add(1);

        // Don't assign LISTENER_TOKEN or CHANNEL_TOKEN
        if self.token_counter.0 < 2 {
            self.token_counter.0 = 2;
        }

        let token = self.token_counter;

        // Remove, deregister and close any existing connection with this token.
        // This shouldn't happen in practice.
        if let Some(connection) = self.connections.remove(&token) {
            ::log::warn!("removing existing connection {} because of token reuse", token.0);

            connection.deregister(poll).close();
        }

        let connection = connection_creator(token);

        self.insert_and_register(poll, token, connection);
    }

    fn insert_and_register(
        &mut self,
        poll: &mut Poll,
        key: Token,
        conn: Connection<NotRegistered>,
    ) {
        self.connections.insert(key, conn.register(poll, key));
    }

    fn remove_and_deregister(
        &mut self,
        poll: &mut Poll,
        key: &Token,
    ) -> Option<Connection<NotRegistered>> {
        if let Some(connection) = self.connections.remove(key) {
            Some(connection.deregister(poll))
        } else {
            None
        }
    }

    fn get_mut(&mut self, key: &Token) -> Option<&mut Connection<Registered>> {
        self.connections.get_mut(key)
    }

    /// Close and remove inactive connections
    fn clean(mut self, poll: &mut Poll) -> Self {
        let now = Instant::now();

        let mut retained_connections = HashMap::default();

        for (token, connection) in self.connections.drain() {
            if connection.valid_until.0 < now {
                connection.deregister(poll).close();
            } else {
                retained_connections.insert(token, connection);
            }
        }

        ConnectionMap {
            connections: retained_connections,
            ..self
        }
    }
}

pub fn run_socket_worker(
    config: Config,
    state: State,
    socket_worker_index: usize,
    socket_worker_statuses: SocketWorkerStatuses,
    poll: Poll,
    in_message_sender: InMessageSender,
    out_message_receiver: OutMessageReceiver,
    tls_config: Arc<rustls::ServerConfig>,
) {
    match create_listener(&config) {
        Ok(listener) => {
            socket_worker_statuses.lock()[socket_worker_index] = Some(Ok(()));

            run_poll_loop(
                config,
                &state,
                socket_worker_index,
                poll,
                in_message_sender,
                out_message_receiver,
                listener,
                tls_config,
            );
        }
        Err(err) => {
            socket_worker_statuses.lock()[socket_worker_index] =
                Some(Err(format!("Couldn't open socket: {:#}", err)));
        }
    }
}

fn run_poll_loop(
    config: Config,
    state: &State,
    socket_worker_index: usize,
    mut poll: Poll,
    in_message_sender: InMessageSender,
    out_message_receiver: OutMessageReceiver,
    listener: ::std::net::TcpListener,
    tls_config: Arc<rustls::ServerConfig>,
) {
    let poll_timeout = Duration::from_micros(config.network.poll_timeout_microseconds);
    let ws_config = WebSocketConfig {
        max_message_size: Some(config.network.websocket_max_message_size),
        max_frame_size: Some(config.network.websocket_max_frame_size),
        max_send_queue: None,
        ..Default::default()
    };

    let mut listener = TcpListener::from_std(listener);
    let mut events = Events::with_capacity(config.network.poll_event_capacity);

    poll.registry()
        .register(&mut listener, LISTENER_TOKEN, Interest::READABLE)
        .unwrap();

    let mut connections = ConnectionMap::default();
    let mut local_responses = Vec::new();

    let mut iter_counter = 0usize;

    loop {
        poll.poll(&mut events, Some(poll_timeout))
            .expect("failed polling");

        let valid_until = ValidUntil::new(config.cleaning.max_connection_age);

        for event in events.iter() {
            let token = event.token();

            if token == LISTENER_TOKEN {
                accept_new_streams(
                    &tls_config,
                    ws_config,
                    socket_worker_index,
                    &mut listener,
                    &mut poll,
                    &mut connections,
                    valid_until,
                );
            } else if token != CHANNEL_TOKEN {
                handle_stream_read_event(
                    &config,
                    state,
                    &mut local_responses,
                    &in_message_sender,
                    &mut poll,
                    &mut connections,
                    token,
                    valid_until,
                );
            }

            send_out_messages(
                &mut poll,
                local_responses.drain(..),
                &out_message_receiver,
                &mut connections,
            );
        }

        // Remove inactive connections, but not every iteration
        if iter_counter % 128 == 0 {
            connections = connections.clean(&mut poll);
        }

        iter_counter = iter_counter.wrapping_add(1);
    }
}

fn accept_new_streams(
    tls_config: &Arc<rustls::ServerConfig>,
    ws_config: WebSocketConfig,
    socket_worker_index: usize,
    listener: &mut TcpListener,
    poll: &mut Poll,
    connections: &mut ConnectionMap,
    valid_until: ValidUntil,
) {
    loop {
        match listener.accept() {
            Ok((stream, _)) => {
                let naive_peer_addr = if let Ok(peer_addr) = stream.peer_addr() {
                    peer_addr
                } else {
                    continue;
                };

                connections.insert_and_register_new(poll, move |token| {
                    let converted_peer_ip = convert_ipv4_mapped_ipv6(naive_peer_addr.ip());

                    let meta = ConnectionMeta {
                        out_message_consumer_id: ConsumerId(socket_worker_index),
                        connection_id: ConnectionId(token.0),
                        naive_peer_addr,
                        converted_peer_ip,
                        pending_scrape_id: None, // FIXME
                    };

                    Connection::new(tls_config.clone(), ws_config, stream, valid_until, meta)
                });
            }
            Err(err) if err.kind() == ErrorKind::WouldBlock => {
                break;
            }
            Err(err) => {
                ::log::info!("error while accepting streams: {}", err);
            }
        }
    }
}

fn handle_stream_read_event(
    config: &Config,
    state: &State,
    local_responses: &mut Vec<(ConnectionMeta, OutMessage)>,
    in_message_sender: &InMessageSender,
    poll: &mut Poll,
    connections: &mut ConnectionMap,
    token: Token,
    valid_until: ValidUntil,
) {
    let access_list_mode = config.access_list.mode;

    if let Some(mut connection) = connections.remove_and_deregister(poll, &token) {
        let message_handler = &mut |meta, message| match message {
            InMessage::AnnounceRequest(ref request)
                if !state
                    .access_list
                    .allows(access_list_mode, &request.info_hash.0) =>
            {
                let out_message = OutMessage::ErrorResponse(ErrorResponse {
                    failure_reason: "Info hash not allowed".into(),
                    action: Some(ErrorResponseAction::Announce),
                    info_hash: Some(request.info_hash),
                });

                local_responses.push((meta, out_message));
            }
            in_message => {
                if let Err(err) = in_message_sender.send((meta, in_message)) {
                    ::log::error!("InMessageSender: couldn't send message: {:?}", err);
                }
            }
        };

        connection.valid_until = valid_until;

        match connection.read(message_handler) {
            Ok(connection) => {
                connections.insert_and_register(poll, token, connection);
            }
            Err(_) => {}
        }
    }
}

/// Read messages from channel, send to peers
fn send_out_messages(
    poll: &mut Poll,
    local_responses: Drain<(ConnectionMeta, OutMessage)>,
    out_message_receiver: &Receiver<(ConnectionMeta, OutMessage)>,
    connections: &mut ConnectionMap,
) {
    let len = out_message_receiver.len();

    for (meta, out_message) in local_responses.chain(out_message_receiver.try_iter().take(len)) {
        let token = Token(meta.connection_id.0);

        let mut remove_connection = false;

        if let Some(connection) = connections.get_mut(&token) {
            if connection.get_meta().naive_peer_addr != meta.naive_peer_addr {
                ::log::warn!(
                    "socket worker error: connection socket addr {} didn't match channel {}. Token: {}.",
                    connection.get_meta().naive_peer_addr,
                    meta.naive_peer_addr,
                    token.0
                );

                remove_connection = true;
            } else if let Err(err) = connection.write(out_message) {
                ::log::debug!("error sending ws message to peer: {}", err);

                remove_connection = true;
            } else {
                ::log::debug!("sent message");
            }
        }

        if remove_connection {
            connections.remove_and_deregister(poll, &token);
        }
    }
}

pub fn create_listener(config: &Config) -> ::anyhow::Result<::std::net::TcpListener> {
    let builder = if config.network.address.is_ipv4() {
        Socket::new(Domain::IPV4, Type::STREAM, Some(Protocol::TCP))
    } else {
        Socket::new(Domain::IPV6, Type::STREAM, Some(Protocol::TCP))
    }
    .context("Couldn't create socket2::Socket")?;

    if config.network.ipv6_only {
        builder
            .set_only_v6(true)
            .context("Couldn't put socket in ipv6 only mode")?
    }

    builder
        .set_nonblocking(true)
        .context("Couldn't put socket in non-blocking mode")?;
    builder
        .set_reuse_port(true)
        .context("Couldn't put socket in reuse_port mode")?;
    builder
        .bind(&config.network.address.into())
        .with_context(|| format!("Couldn't bind socket to address {}", config.network.address))?;
    builder
        .listen(128)
        .context("Couldn't listen for connections on socket")?;

    Ok(builder.into())
}
