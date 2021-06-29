use std::time::{Duration, Instant};
use std::io::{ErrorKind, Cursor};
use std::sync::Arc;
use std::vec::Drain;

use hashbrown::HashMap;
use log::{info, debug};
use mio::{Events, Poll, Interest, Token};
use mio::net::TcpListener;

use aquatic_http_protocol::response::*;

use crate::common::*;
use crate::config::Config;

pub mod connection;
pub mod utils;

use connection::*;
use utils::*;


const CONNECTION_CLEAN_INTERVAL: usize = 2 ^ 22;


pub fn run_socket_worker(
    config: Config,
    socket_worker_index: usize,
    socket_worker_statuses: SocketWorkerStatuses,
    request_channel_sender: RequestChannelSender,
    response_channel_receiver: ResponseChannelReceiver,
    opt_tls_config: &Option<Arc<rustls::ServerConfig>>,
    poll: Poll,
){
    match create_listener(config.network.address, config.network.ipv6_only){
        Ok(listener) => {
            socket_worker_statuses.lock()[socket_worker_index] = Some(Ok(()));

            run_poll_loop(
                config,
                socket_worker_index,
                request_channel_sender,
                response_channel_receiver,
                listener,
                opt_tls_config,
                poll,
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
    request_channel_sender: RequestChannelSender,
    response_channel_receiver: ResponseChannelReceiver,
    listener: ::std::net::TcpListener,
    opt_tls_config: &Option<Arc<rustls::ServerConfig>>,
    mut poll: Poll,
){
    let poll_timeout = Duration::from_micros(
        config.network.poll_timeout_microseconds
    );

    let mut listener = TcpListener::from_std(listener);
    let mut events = Events::with_capacity(config.network.poll_event_capacity);

    poll.registry()
        .register(&mut listener, Token(0), Interest::READABLE)
        .unwrap();

    let mut connections: ConnectionMap = HashMap::new();

    let mut poll_token_counter = Token(0usize);
    let mut iter_counter = 0usize;

    let mut response_buffer = [0u8; 4096];
    let mut response_buffer = Cursor::new(&mut response_buffer[..]);
    let mut local_responses = Vec::new();

    loop {
        poll.poll(&mut events, Some(poll_timeout))
            .expect("failed polling");

        for event in events.iter(){
            let token = event.token();

            if token == LISTENER_TOKEN {
                accept_new_streams(
                    &config,
                    &mut listener,
                    &mut poll,
                    &mut connections,
                    &mut poll_token_counter,
                    &opt_tls_config,
                );
            } else if token != CHANNEL_TOKEN {
                let opt_connection = connections.get_mut(&token);

                if let Some(connection) = opt_connection {
                    let status = connection.handle_read_event(
                        socket_worker_index,
                        &request_channel_sender,
                        &mut local_responses,
                        token
                    );

                    if let ConnectionPollStatus::Remove = status {
                        ::std::mem::drop(connection);

                        remove_connection(&mut poll, &mut connections, &token);
                    } else {
                        connection.valid_until = ValidUntil::new(config.cleaning.max_connection_age);
                    }
                }
            }

            // Send responses for each event. Channel token is not interesting
            // by itself, but is just for making sure responses are sent even
            // if no new connects / requests come in.
            send_responses(
                &config,
                &mut poll,
                &mut response_buffer,
                local_responses.drain(..),
                &response_channel_receiver,
                &mut connections
            );
        }

        // Remove inactive connections, but not every iteration
        if iter_counter % CONNECTION_CLEAN_INTERVAL == 0 {
            remove_inactive_connections(&mut poll, &mut connections);
        }

        iter_counter = iter_counter.wrapping_add(1);
    }
}


fn accept_new_streams(
    config: &Config,
    listener: &mut TcpListener,
    poll: &mut Poll,
    connections: &mut ConnectionMap,
    poll_token_counter: &mut Token,
    opt_tls_config: &Option<Arc<rustls::ServerConfig>>,
){
    let valid_until = ValidUntil::new(config.cleaning.max_connection_age);

    loop {
        match listener.accept(){
            Ok((mut stream, _)) => {
                poll_token_counter.0 = poll_token_counter.0.wrapping_add(1);

                // Skip listener and channel tokens
                if poll_token_counter.0 < 2 {
                    poll_token_counter.0 = 2;
                }

                let token = *poll_token_counter;

                // Remove connection if it exists (which is unlikely)
                remove_connection(poll, connections, poll_token_counter);
                
                match stream.peer_addr() {
                    Ok(peer_addr) => {
                        poll.registry()
                            .register(&mut stream, token, Interest::READABLE)
                            .unwrap();

                        let tls_session = opt_tls_config.as_ref()
                            .map(|config| rustls::ServerSession::new(&config));

                        let connection = Connection::new(stream, peer_addr, valid_until, tls_session);

                        connections.insert(token, connection);
                    },
                    Err(err) => {
                        ::log::info!("Couln't get stream peer_addr: {}", err);
                    }
                }
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

/// Read responses from channel, send to peers
pub fn send_responses(
    config: &Config,
    poll: &mut Poll,
    buffer: &mut Cursor<&mut [u8]>,
    local_responses: Drain<(ConnectionMeta, Response)>,
    channel_responses: &ResponseChannelReceiver,
    connections: &mut ConnectionMap,
){
    let channel_responses_len = channel_responses.len();
    let channel_responses_drain = channel_responses.try_iter()
        .take(channel_responses_len);

    for (meta, response) in local_responses.chain(channel_responses_drain){
        if let Some(connection) = connections.get_mut(&meta.poll_token) {
            if connection.peer_addr != meta.peer_addr {
                info!("socket worker error: peer socket addrs didn't match");

                continue;
            }

            buffer.set_position(0);

            let bytes_written = response.write(buffer).unwrap();

            match connection.send_response(&buffer.get_mut()[..bytes_written]){
                Ok(()) => {
                    ::log::debug!(
                        "sent response: {:?} with response string {}",
                        response,
                        String::from_utf8_lossy(&buffer.get_ref()[..bytes_written])
                    );

                    if !config.network.keep_alive {
                        remove_connection(poll, connections, &meta.poll_token);
                    }
                },
                Err(err) if err.kind() == ErrorKind::WouldBlock => {
                    debug!("send response: would block");
                },
                Err(err) => {
                    info!("error sending response: {}", err);

                    remove_connection(poll, connections, &meta.poll_token);
                },
            }
        }
    }
}


// Close and remove inactive connections
pub fn remove_inactive_connections(
    poll: &mut Poll,
    connections: &mut ConnectionMap,
){
    let now = Instant::now();

    connections.retain(|_, connection| {
        let keep = connection.valid_until.0 >= now;

        if !keep {
            if let Err(err) = connection.deregister(poll){
                ::log::error!("deregister connection error: {}", err);
            }
        }

        keep
    });

    connections.shrink_to_fit();
}


fn remove_connection(
    poll: &mut Poll,
    connections: &mut ConnectionMap,
    connection_token: &Token,
){
    if let Some(mut connection) = connections.remove(connection_token){
        if let Err(err) = connection.deregister(poll){
            ::log::error!("deregister connection error: {}", err);
        }
    }
}