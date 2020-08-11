use std::time::Duration;
use std::io::ErrorKind;

use crossbeam_channel::Receiver;
use hashbrown::HashMap;
use log::{info, debug, error};
use native_tls::TlsAcceptor;
use mio::{Events, Poll, Interest, Token};
use mio::net::TcpListener;
use tungstenite::protocol::WebSocketConfig;

use aquatic_common::convert_ipv4_mapped_ipv6;
use aquatic_ws_protocol::*;

use crate::common::*;
use crate::config::Config;

pub mod connection;
pub mod utils;

use connection::*;
use utils::*;


pub fn run_socket_worker(
    config: Config,
    socket_worker_index: usize,
    socket_worker_statuses: SocketWorkerStatuses,
    poll: Poll,
    in_message_sender: InMessageSender,
    out_message_receiver: OutMessageReceiver,
    opt_tls_acceptor: Option<TlsAcceptor>,
){
    match create_listener(&config){
        Ok(listener) => {
            socket_worker_statuses.lock()[socket_worker_index] = Some(Ok(()));

            run_poll_loop(
                config,
                socket_worker_index,
                poll,
                in_message_sender,
                out_message_receiver,
                listener,
                opt_tls_acceptor
            );
        },
        Err(err) => {
            socket_worker_statuses.lock()[socket_worker_index] = Some(
                Err(format!("Couldn't open socket: {:#}", err))
            );
        }
    }
}


pub fn run_poll_loop(
    config: Config,
    socket_worker_index: usize,
    mut poll: Poll,
    in_message_sender: InMessageSender,
    out_message_receiver: OutMessageReceiver,
    listener: ::std::net::TcpListener,
    opt_tls_acceptor: Option<TlsAcceptor>,
){
    let poll_timeout = Duration::from_micros(
        config.network.poll_timeout_microseconds
    );
    let ws_config = WebSocketConfig {
        max_message_size: Some(config.network.websocket_max_message_size),
        max_frame_size: Some(config.network.websocket_max_frame_size),
        max_send_queue: None,
    };

    let mut listener = TcpListener::from_std(listener);
    let mut events = Events::with_capacity(config.network.poll_event_capacity);

    poll.registry()
        .register(&mut listener, LISTENER_TOKEN, Interest::READABLE)
        .unwrap();

    let mut connections: ConnectionMap = HashMap::new();

    let mut poll_token_counter = Token(0usize);
    let mut iter_counter = 0usize;

    loop {
        poll.poll(&mut events, Some(poll_timeout))
            .expect("failed polling");
        
        let valid_until = ValidUntil::new(config.cleaning.max_connection_age);

        for event in events.iter(){
            let token = event.token();

            if token == LISTENER_TOKEN {
                accept_new_streams(
                    ws_config,
                    &mut listener,
                    &mut poll,
                    &mut connections,
                    valid_until,
                    &mut poll_token_counter,
                );
            } else if token != CHANNEL_TOKEN {
                run_handshakes_and_read_messages(
                    socket_worker_index,
                    &in_message_sender,
                    &opt_tls_acceptor,
                    &mut poll,
                    &mut connections,
                    token,
                    valid_until,
                );
            }

            send_out_messages(
                &mut poll,
                &out_message_receiver,
                &mut connections
            );
        }

        // Remove inactive connections, but not every iteration
        if iter_counter % 128 == 0 {
            remove_inactive_connections(&mut connections);
        }

        iter_counter = iter_counter.wrapping_add(1);
    }
}


fn accept_new_streams(
    ws_config: WebSocketConfig,
    listener: &mut TcpListener,
    poll: &mut Poll,
    connections: &mut ConnectionMap,
    valid_until: ValidUntil,
    poll_token_counter: &mut Token,
){
    loop {
        match listener.accept(){
            Ok((mut stream, _)) => {
                poll_token_counter.0 = poll_token_counter.0.wrapping_add(1);

                if poll_token_counter.0 < 2 {
                    poll_token_counter.0 = 2;
                }

                let token = *poll_token_counter;

                remove_connection_if_exists(poll, connections, token);

                poll.registry()
                    .register(&mut stream, token, Interest::READABLE)
                    .unwrap();

                let connection = Connection::new(ws_config, valid_until, stream);

                connections.insert(token, connection);
            },
            Err(err) => {
                if err.kind() == ErrorKind::WouldBlock {
                    break
                }

                info!("error while accepting streams: {}", err);
            }
        }
    }
}


/// On the stream given by poll_token, get TLS (if requested) and tungstenite
/// up and running, then read messages and pass on through channel.
pub fn run_handshakes_and_read_messages(
    socket_worker_index: usize,
    in_message_sender: &InMessageSender,
    opt_tls_acceptor: &Option<TlsAcceptor>, // If set, run TLS
    poll: &mut Poll,
    connections: &mut ConnectionMap,
    poll_token: Token,
    valid_until: ValidUntil,
){
    loop {
        if let Some(established_ws) = connections.get_mut(&poll_token)
            .map(|c| { // Ugly but works
                c.valid_until = valid_until;

                c
            })
            .and_then(Connection::get_established_ws)
        {
            use ::tungstenite::Error::Io;

            match established_ws.ws.read_message(){
                Ok(ws_message) => {
                    if let Ok(in_message) = InMessage::from_ws_message(ws_message){
                        let naive_peer_addr = established_ws.peer_addr;
                        let converted_peer_ip = convert_ipv4_mapped_ipv6(
                            naive_peer_addr.ip()
                        );

                        let meta = ConnectionMeta {
                            worker_index: socket_worker_index,
                            poll_token,
                            naive_peer_addr,
                            converted_peer_ip,
                        };

                        debug!("read message");
    
                        if let Err(err) = in_message_sender
                            .send((meta, in_message))
                        {
                            error!(
                                "InMessageSender: couldn't send message: {:?}",
                                err
                            );
                        }
                    }
                },
                Err(Io(err)) if err.kind() == ErrorKind::WouldBlock => {
                    break;
                },
                Err(tungstenite::Error::ConnectionClosed) => {
                    remove_connection_if_exists(poll, connections, poll_token);

                    break
                },
                Err(err) => {
                    info!("error reading messages: {}", err);
    
                    remove_connection_if_exists(poll, connections, poll_token);
    
                    break;
                }
            }
        } else if let Some(connection) = connections.remove(&poll_token){
            let (opt_new_connection, stop_loop) = connection.advance_handshakes(
                opt_tls_acceptor,
                valid_until
            );

            if let Some(connection) = opt_new_connection {
                connections.insert(poll_token, connection);
            }

            if stop_loop {
                break;
            }
        } else {
            break
        }
    }
}


/// Read messages from channel, send to peers
pub fn send_out_messages(
    poll: &mut Poll,
    out_message_receiver: &Receiver<(ConnectionMeta, OutMessage)>,
    connections: &mut ConnectionMap,
){
    let len = out_message_receiver.len();

    for (meta, out_message) in out_message_receiver.try_iter().take(len){
        let opt_established_ws = connections.get_mut(&meta.poll_token)
            .and_then(Connection::get_established_ws);
        
        if let Some(established_ws) = opt_established_ws {
            if established_ws.peer_addr != meta.naive_peer_addr {
                info!("socket worker error: peer socket addrs didn't match");

                continue;
            }
        
            use ::tungstenite::Error::Io;

            let ws_message = out_message.to_ws_message();

            match established_ws.ws.write_message(ws_message){
                Ok(()) => {
                    debug!("sent message");
                },
                Err(Io(err)) if err.kind() == ErrorKind::WouldBlock => {},
                Err(tungstenite::Error::ConnectionClosed) => {
                    remove_connection_if_exists(
                        poll,
                        connections,
                        meta.poll_token
                    );
                },
                Err(err) => {
                    info!("error writing ws message: {}", err);

                    remove_connection_if_exists(
                        poll,
                        connections,
                        meta.poll_token
                    );
                },
            }
        }
    }
}