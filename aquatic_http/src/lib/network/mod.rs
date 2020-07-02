pub mod connection;
pub mod utils;

use std::time::Duration;
use std::io::ErrorKind;

use hashbrown::HashMap;
use log::{info, debug, error};
use native_tls::TlsAcceptor;
use mio::{Events, Poll, Interest, Token};
use mio::net::TcpListener;

use aquatic_common_tcp::network::create_listener;

use crate::common::*;
use crate::config::Config;
use crate::protocol::*;

use connection::*;
use utils::*;



fn accept_new_streams(
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

                if poll_token_counter.0 == 0 {
                    poll_token_counter.0 = 1;
                }

                let token = *poll_token_counter;

                remove_connection_if_exists(connections, token);

                poll.registry()
                    .register(&mut stream, token, Interest::READABLE)
                    .unwrap();

                let connection = Connection::new(valid_until, stream);

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

// will be almost identical to ws version
pub fn run_socket_worker(
    config: Config,
    socket_worker_index: usize,
    socket_worker_statuses: SocketWorkerStatuses,
    request_channel_sender: RequestChannelSender,
    response_channel_receiver: ResponseChannelReceiver,
    opt_tls_acceptor: Option<TlsAcceptor>,
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


// will be almost identical to ws version
pub fn run_poll_loop(
    config: Config,
    socket_worker_index: usize,
    request_channel_sender: RequestChannelSender,
    response_channel_receiver: ResponseChannelReceiver,
    listener: ::std::net::TcpListener,
    opt_tls_acceptor: Option<TlsAcceptor>,
){
    let poll_timeout = Duration::from_millis(
        config.network.poll_timeout_milliseconds
    );

    let mut listener = TcpListener::from_std(listener);
    let mut poll = Poll::new().expect("create poll");
    let mut events = Events::with_capacity(config.network.poll_event_capacity);

    poll.registry()
        .register(&mut listener, Token(0), Interest::READABLE)
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

            if token.0 == 0 {
                accept_new_streams(
                    &mut listener,
                    &mut poll,
                    &mut connections,
                    valid_until,
                    &mut poll_token_counter,
                );
            } else {
                run_handshake_and_read_requests(
                    socket_worker_index,
                    &request_channel_sender,
                    &opt_tls_acceptor,
                    &mut connections,
                    token,
                    valid_until,
                );
            }
        }

        send_responses(
            response_channel_receiver.drain(),
            &mut connections
        );

        // Remove inactive connections, but not every iteration
        if iter_counter % 128 == 0 {
            remove_inactive_connections(&mut connections);
        }

        iter_counter = iter_counter.wrapping_add(1);
    }
}



/// On the stream given by poll_token, get TLS (if requested) and tungstenite
/// up and running, then read messages and pass on through channel.
pub fn run_handshake_and_read_requests(
    socket_worker_index: usize,
    request_channel_sender: &RequestChannelSender,
    opt_tls_acceptor: &Option<TlsAcceptor>, // If set, run TLS
    connections: &mut ConnectionMap,
    poll_token: Token,
    valid_until: ValidUntil,
){
    loop {
        if let Some(established_connection) = connections.get_mut(&poll_token)
            .and_then(Connection::get_established)
        {
            match established_connection.parse_request(){
                Ok(request) => {
                    let meta = ConnectionMeta {
                        worker_index: socket_worker_index,
                        poll_token,
                        peer_addr: established_connection.peer_addr
                    };

                    debug!("read request, sending to handler");

                    if let Err(err) = request_channel_sender
                        .send((meta, request))
                    {
                        error!(
                            "RequestChannelSender: couldn't send message: {:?}",
                            err
                        );
                    }

                    break
                },
                Err(RequestParseError::NeedMoreData) => {
                    info!("need more data");

                    break;
                },
                Err(RequestParseError::Io(err)) => {
                    info!("error reading request: {}", err);
    
                    remove_connection_if_exists(connections, poll_token);
    
                    break;
                },
                Err(e) => {
                    info!("error reading request: {:?}", e);

                    remove_connection_if_exists(connections, poll_token);
    
                    break;
                },
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
        }
    }
}


/// Read messages from channel, send to peers
pub fn send_responses(
    response_channel_receiver: ::flume::Drain<(ConnectionMeta, Response)>,
    connections: &mut ConnectionMap,
){
    for (meta, response) in response_channel_receiver {
        let opt_established = connections.get_mut(&meta.poll_token)
            .and_then(Connection::get_established);
        
        if let Some(established) = opt_established {
            if established.peer_addr != meta.peer_addr {
                info!("socket worker error: peer socket addrs didn't match");

                continue;
            }

            match established.send_response(&response.to_bytes()){
                Ok(()) => {
                    debug!("sent response");

                    remove_connection_if_exists(
                        connections,
                        meta.poll_token
                    );
                },
                Err(err) if err.kind() == ErrorKind::WouldBlock => {
                    debug!("send response: would block");
                },
                Err(err) => {
                    info!("error sending response: {}", err);

                    remove_connection_if_exists(
                        connections,
                        meta.poll_token
                    );
                },
            }
        }
    }
}