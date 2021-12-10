use std::io::ErrorKind;
use std::sync::Arc;
use std::time::Duration;
use std::vec::Drain;

use aquatic_common::access_list::AccessListQuery;
use crossbeam_channel::Receiver;
use hashbrown::HashMap;
use mio::net::TcpListener;
use mio::{Events, Interest, Poll, Token};
use tungstenite::protocol::WebSocketConfig;

use aquatic_common::convert_ipv4_mapped_ipv6;
use aquatic_ws_protocol::*;

use crate::common::*;
use crate::config::Config;

use self::connection::Registered;

use super::common::*;

pub mod connection;
pub mod utils;

use connection::Connection;
use utils::*;

type ConnectionMap = HashMap<Token, Connection<Registered>>;

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

pub fn run_poll_loop(
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

    let mut connections: ConnectionMap = HashMap::new();
    let mut local_responses = Vec::new();

    let mut poll_token_counter = Token(0usize);
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
                    &mut poll_token_counter,
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
            connections = remove_inactive_connections(connections, &mut poll);
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
    poll_token_counter: &mut Token,
) {
    loop {
        match listener.accept() {
            Ok((stream, _)) => {
                poll_token_counter.0 = poll_token_counter.0.wrapping_add(1);

                if poll_token_counter.0 < 2 {
                    poll_token_counter.0 = 2;
                }

                let token = *poll_token_counter;

                if let Some(connection) = connections.remove(&token) {
                    connection.deregister(poll).close();
                }

                let naive_peer_addr = if let Ok(peer_addr) = stream.peer_addr() {
                    peer_addr
                } else {
                    continue;
                };

                let converted_peer_ip = convert_ipv4_mapped_ipv6(naive_peer_addr.ip());

                let meta = ConnectionMeta {
                    out_message_consumer_id: ConsumerId(socket_worker_index),
                    connection_id: ConnectionId(token.0),
                    naive_peer_addr,
                    converted_peer_ip,
                    pending_scrape_id: None, // FIXME
                };

                let connection =
                    Connection::new(tls_config.clone(), ws_config, stream, valid_until, meta)
                        .register(poll, token);

                connections.insert(token, connection);
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

pub fn handle_stream_read_event(
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

    if let Some(mut connection) = connections.remove(&token) {
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

        match connection.deregister(poll).read(message_handler) {
            Ok(connection) => {
                connections.insert(token, connection.register(poll, token));
            }
            Err(err) => {
                ::log::info!("Connection error: {}", err);
            }
        }
    }
}

/// Read messages from channel, send to peers
pub fn send_out_messages(
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
                ::log::info!("socket worker error: peer socket addrs didn't match");

                remove_connection = true;
            } else if let Err(err) = connection.write(out_message) {
                ::log::info!("error sending ws message to peer: {}", err);

                remove_connection = true;
            }
        }

        if remove_connection {
            if let Some(connection) = connections.remove(&token) {
                connection.deregister(poll).close();
            }
        }
    }
}
