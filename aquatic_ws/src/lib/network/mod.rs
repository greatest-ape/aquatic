use std::time::Duration;
use std::io::ErrorKind;

use tungstenite::WebSocket;
use tungstenite::handshake::{HandshakeError, server::ServerHandshake};
use hashbrown::HashMap;
use native_tls::{TlsAcceptor, TlsStream};

use mio::{Events, Poll, Interest, Token};
use mio::net::{TcpListener, TcpStream};

use crate::common::*;
use crate::config::Config;
use crate::protocol::*;

pub mod common;
pub mod utils;

use common::*;
use utils::*;


pub fn run_socket_worker(
    config: Config,
    socket_worker_index: usize,
    in_message_sender: InMessageSender,
    out_message_receiver: OutMessageReceiver,
    use_tls: bool
){
    let poll_timeout = Duration::from_millis(
        config.network.poll_timeout_milliseconds
    );

    let mut listener = TcpListener::from_std(create_listener(&config));
    let mut poll = Poll::new().expect("create poll");
    let mut events = Events::with_capacity(config.network.poll_event_capacity);

    poll.registry()
        .register(&mut listener, Token(0), Interest::READABLE)
        .unwrap();

    let opt_tls_acceptor = if use_tls {
        Some(create_tls_acceptor(&config))
    } else {
        None
    };

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
            } else if event.is_readable(){
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
        }

        send_out_messages(
            out_message_receiver.drain(),
            &mut poll,
            &mut connections
        );

        // Remove inactive connections, but not every iteration
        if iter_counter % 128 == 0 {
            remove_inactive_connections(&mut poll, &mut connections);
        }

        iter_counter = iter_counter.wrapping_add(1);
    }
}


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

                remove_connection_if_exists(poll, connections, token);

                poll.registry()
                    .register(&mut stream, token, Interest::READABLE)
                    .unwrap();

                let connection = Connection {
                    valid_until,
                    stage: ConnectionStage::TcpStream(stream)
                };

                connections.insert(token, connection);
            },
            Err(err) => {
                if err.kind() == ErrorKind::WouldBlock {
                    break
                }

                eprint!("{}", err);
            }
        }
    }
}


pub fn handle_tls_handshake_result(
    connections: &mut ConnectionMap,
    poll_token: Token,
    valid_until: ValidUntil,
    result: Result<TlsStream<TcpStream>, native_tls::HandshakeError<TcpStream>>,
) -> bool {
    match result {
        Ok(stream) => {
            println!("handshake established");

            let connection = Connection {
                valid_until,
                stage: ConnectionStage::TlsStream(stream)
            };

            connections.insert(poll_token, connection);

            false
        },
        Err(native_tls::HandshakeError::WouldBlock(handshake)) => {
            println!("interrupted");

            let connection = Connection {
                valid_until,
                stage: ConnectionStage::TlsMidHandshake(handshake),
            };

            connections.insert(poll_token, connection);

            true
        },
        Err(native_tls::HandshakeError::Failure(err)) => {
            dbg!(err);

            false
        }
    }
}


pub fn handle_ws_handshake_no_tls_result(
    connections: &mut ConnectionMap,
    poll_token: Token,
    valid_until: ValidUntil,
    result: Result<WebSocket<TcpStream>, HandshakeError<ServerHandshake<TcpStream, DebugCallback>>> ,
) -> bool {
    match result {
        Ok(mut ws) => {
            println!("handshake established");

            let peer_addr = ws.get_mut().peer_addr().unwrap();

            let established_ws = EstablishedWs {
                ws,
                peer_addr,
            };

            let connection = Connection {
                valid_until,
                stage: ConnectionStage::EstablishedWsNoTls(established_ws)
            };

            connections.insert(poll_token, connection);

            false
        },
        Err(HandshakeError::Interrupted(handshake)) => {
            println!("interrupted");

            let connection = Connection {
                valid_until,
                stage: ConnectionStage::WsHandshakeNoTls(handshake),
            };

            connections.insert(poll_token, connection);

            true
        },
        Err(HandshakeError::Failure(err)) => {
            dbg!(err);

            false
        }
    }
}


pub fn handle_ws_handshake_tls_result(
    connections: &mut ConnectionMap,
    poll_token: Token,
    valid_until: ValidUntil,
    result: Result<WebSocket<Stream>, HandshakeError<ServerHandshake<Stream, DebugCallback>>> ,
) -> bool {
    match result {
        Ok(mut ws) => {
            println!("handshake established");

            let peer_addr = ws.get_mut().get_mut().peer_addr().unwrap();

            let established_ws = EstablishedWs {
                ws,
                peer_addr,
            };

            let connection = Connection {
                valid_until,
                stage: ConnectionStage::EstablishedWsTls(established_ws)
            };

            connections.insert(poll_token, connection);

            false
        },
        Err(HandshakeError::Interrupted(handshake)) => {
            println!("interrupted");

            let connection = Connection {
                valid_until,
                stage: ConnectionStage::WsHandshakeTls(handshake),
            };

            connections.insert(poll_token, connection);

            true
        },
        Err(HandshakeError::Failure(err)) => {
            dbg!(err);

            false
        }
    }
}


// Macro hack to not have to write the following twice in
// `run_handshakes_and_read_messages` (putting it in a function causes error
// because of multiple mutable references)
macro_rules! read_ws_messages {
    (
        $socket_worker_index: ident,
        $in_message_sender: ident,
        $poll: ident,
        $connections: ident,
        $poll_token: ident,
        $established_ws: ident
    ) => {
        println!("conn established");

        match $established_ws.ws.read_message(){
            Ok(ws_message) => {
                dbg!(ws_message.clone());

                if let Some(in_message) = InMessage::from_ws_message(ws_message){
                    dbg!(in_message.clone());

                    let meta = ConnectionMeta {
                        worker_index: $socket_worker_index,
                        poll_token: $poll_token,
                        peer_addr: $established_ws.peer_addr
                    };

                    $in_message_sender.send((meta, in_message));
                }
            },
            Err(tungstenite::Error::Io(err)) => {
                if err.kind() == ErrorKind::WouldBlock {
                    break;
                }

                remove_connection_if_exists($poll, $connections, $poll_token);

                eprint!("{}", err);

                break;
            },
            Err(tungstenite::Error::ConnectionClosed) => {
                remove_connection_if_exists($poll, $connections, $poll_token);

                break;
            },
            Err(err) => {
                dbg!(err);

                remove_connection_if_exists($poll, $connections, $poll_token);

                break;
            }
        } 
    };
}


/// Get TLS (if requested) and tungstenite up and running, then read messages
pub fn run_handshakes_and_read_messages(
    socket_worker_index: usize,
    in_message_sender: &InMessageSender,
    opt_tls_acceptor: &Option<TlsAcceptor>, // If set, run TLS
    poll: &mut Poll,
    connections: &mut ConnectionMap,
    poll_token: Token,
    valid_until: ValidUntil,
){
    println!("poll_token: {}", poll_token.0);

    loop {
        let established = match connections.get(&poll_token).map(|c| &c.stage){
            Some(stage) => stage.is_established(),
            None => break,
        };

        if !established {
            let conn = connections.remove(&poll_token).unwrap();

            match conn.stage {
                ConnectionStage::TcpStream(stream) => {
                    if let Some(tls_acceptor) = opt_tls_acceptor {
                        let stop_loop = handle_tls_handshake_result(
                            connections,
                            poll_token,
                            valid_until,
                            tls_acceptor.accept(stream)
                        );
    
                        if stop_loop {
                            break;
                        }
                    } else {
                        let handshake_result = ::tungstenite::server::accept_hdr(
                            stream,
                            DebugCallback
                        );

                        let stop_loop = handle_ws_handshake_no_tls_result(
                            connections,
                            poll_token,
                            valid_until,
                            handshake_result
                        );

                        if stop_loop {
                            break;
                        }
                    }
                },
                ConnectionStage::TlsMidHandshake(handshake) => {
                    let stop_loop = handle_tls_handshake_result(
                        connections,
                        poll_token,
                        valid_until,
                        handshake.handshake()
                    );

                    if stop_loop {
                        break;
                    }
                },
                ConnectionStage::TlsStream(stream) => {
                    let handshake_result = ::tungstenite::server::accept_hdr(
                        stream,
                        DebugCallback
                    );

                    let stop_loop = handle_ws_handshake_tls_result(
                        connections,
                        poll_token,
                        valid_until,
                        handshake_result
                    );

                    if stop_loop {
                        break;
                    }
                },
                ConnectionStage::WsHandshakeNoTls(handshake) => {
                    let stop_loop = handle_ws_handshake_no_tls_result(
                        connections,
                        poll_token,
                        valid_until,
                        handshake.handshake()
                    );

                    if stop_loop {
                        break;
                    }
                },
                ConnectionStage::WsHandshakeTls(handshake) => {
                    let stop_loop = handle_ws_handshake_tls_result(
                        connections,
                        poll_token,
                        valid_until,
                        handshake.handshake()
                    );

                    if stop_loop {
                        break;
                    }
                },
                ConnectionStage::EstablishedWsNoTls(_) => unreachable!(),
                ConnectionStage::EstablishedWsTls(_) => unreachable!(),
            }
        } else {
            match connections.get_mut(&poll_token){
                Some(Connection{
                    stage: ConnectionStage::EstablishedWsNoTls(established_ws),
                    ..
                }) => {
                    read_ws_messages!(
                        socket_worker_index,
                        in_message_sender,
                        poll,
                        connections,
                        poll_token,
                        established_ws
                    );
                },
                Some(Connection{
                    stage: ConnectionStage::EstablishedWsTls(established_ws),
                    ..
                }) => {
                    read_ws_messages!(
                        socket_worker_index,
                        in_message_sender,
                        poll,
                        connections,
                        poll_token,
                        established_ws
                    );
                },
                _ => ()
            }
        }
    }
}


/// Read messages from channel, send to peers
pub fn send_out_messages(
    out_message_receiver: ::flume::Drain<(ConnectionMeta, OutMessage)>,
    poll: &mut Poll,
    connections: &mut ConnectionMap,
){
    for (meta, out_message) in out_message_receiver {
        let opt_stage = connections
            .get_mut(&meta.poll_token)
            .map(|v| &mut v.stage);
        
        use ::tungstenite::Error::Io;
        
        // Exactly the same for both established stages
        match opt_stage {
            Some(ConnectionStage::EstablishedWsNoTls(connection)) => {
                if connection.peer_addr != meta.peer_addr {
                    eprintln!("socket worker: peer socket addrs didn't match");

                    continue;
                }

                dbg!(out_message.clone());

                match connection.ws.write_message(out_message.to_ws_message()){
                    Ok(()) => {},
                    Err(Io(err)) if err.kind() == ErrorKind::WouldBlock => {
                        continue;
                    },
                    Err(err) => {
                        dbg!(err);

                        remove_connection_if_exists(
                            poll,
                            connections,
                            meta.poll_token
                        );
                    },
                }
            },
            Some(ConnectionStage::EstablishedWsTls(connection)) => {
                if connection.peer_addr != meta.peer_addr {
                    eprintln!("socket worker: peer socket addrs didn't match");

                    continue;
                }

                dbg!(out_message.clone());

                match connection.ws.write_message(out_message.to_ws_message()){
                    Ok(()) => {},
                    Err(Io(err)) if err.kind() == ErrorKind::WouldBlock => {
                        continue;
                    },
                    Err(err) => {
                        dbg!(err);

                        remove_connection_if_exists(
                            poll,
                            connections,
                            meta.poll_token
                        );
                    },
                }
            },
            _ => {},
        }
    }
}