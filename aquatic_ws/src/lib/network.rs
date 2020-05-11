use std::net::{SocketAddr};
use std::time::{Duration, Instant};
use std::io::ErrorKind;

use tungstenite::WebSocket;
use tungstenite::handshake::{MidHandshake, HandshakeError, server::{ServerHandshake, NoCallback}};
use hashbrown::HashMap;

use mio::{Events, Poll, Interest, Token};
use mio::net::{TcpListener, TcpStream};

use crate::common::*;
use crate::protocol::*;


pub struct Connection {
    valid_until: ValidUntil,
    stage: ConnectionStage,
}


pub enum ConnectionStage {
    Stream(TcpStream),
    MidHandshake(MidHandshake<ServerHandshake<TcpStream, DebugCallback>>),
    Established(PeerConnection),
}


pub struct PeerConnection {
    pub ws: WebSocket<TcpStream>,
    pub peer_socket_addr: SocketAddr,
    pub valid_until: ValidUntil,
}


pub type ConnectionMap = HashMap<Token, Connection>;


#[derive(Clone, Copy, Debug)]
pub struct DebugCallback;

impl ::tungstenite::handshake::server::Callback for DebugCallback {
    fn on_request(
        self,
        request: &::tungstenite::handshake::server::Request,
        response: ::tungstenite::handshake::server::Response,
    ) -> Result<::tungstenite::handshake::server::Response, ::tungstenite::handshake::server::ErrorResponse> {
        println!("request: {:#?}", request);
        println!("response: {:#?}", response);

        Ok(response)
    }
}


// Close and remove inactive connections
pub fn remove_inactive_connections(
    poll: &mut Poll,
    connections: &mut ConnectionMap,
){
    let now = Instant::now();

    connections.retain(|_, connection| {
        if connection.valid_until.0 < now {
            match connection.stage {
                ConnectionStage::Stream(ref mut stream) => {
                    poll.registry()
                        .deregister(stream)
                        .unwrap();
                },
                ConnectionStage::MidHandshake(ref mut handshake) => {
                    poll.registry()
                        .deregister(handshake.get_mut().get_mut())
                        .unwrap();
                },
                ConnectionStage::Established(ref mut peer_connection) => {
                    peer_connection.ws.close(None).unwrap();

                    // Needs to be done after ws.close()
                    if let Err(err) = peer_connection.ws.write_pending(){
                        dbg!(err);
                    }

                    poll.registry()
                        .deregister(peer_connection.ws.get_mut())
                        .unwrap();
                },
            }

            println!("closing connection, it is inactive");

            false
        } else {
            println!("keeping connection, it is still active");

            true
        }
    });
}


pub fn run_socket_worker(
    address: SocketAddr,
    socket_worker_index: usize,
    in_message_sender: InMessageSender,
    out_message_receiver: OutMessageReceiver,
){
    let poll_timeout = Duration::from_millis(50); // FIXME: config

    let mut listener = TcpListener::bind(address).unwrap();
    let mut poll = Poll::new().expect("create poll");
    let mut events = Events::with_capacity(1024); // FIXME: config

    poll.registry()
        .register(&mut listener, Token(0), Interest::READABLE)
        .unwrap();

    let mut connections: ConnectionMap = HashMap::new();

    let mut poll_token_counter = Token(0usize);
    let mut iter_counter = 0usize;

    loop {
        poll.poll(&mut events, Some(poll_timeout))
            .expect("failed polling");
        
        let valid_until = ValidUntil::new(600); // FIXME: config

        for event in events.iter(){
            let token = event.token();

            if token.0 == 0 {
                accept_new_streams(
                    &mut listener,
                    &mut poll,
                    &mut connections,
                    valid_until,
                    &mut poll_token_counter
                );
            } else if event.is_readable(){
                run_handshake_and_read_messages(
                    socket_worker_index,
                    &in_message_sender,
                    &mut poll,
                    &mut connections,
                    token,
                    valid_until
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


/// FIXME: close channel if not closed?
fn remove_connection_if_exists(
    poll: &mut Poll,
    connections: &mut ConnectionMap,
    token: Token,
){
    if let Some(connection) = connections.remove(&token){
        match connection.stage {
            ConnectionStage::Stream(mut stream) => {
                poll.registry()
                    .deregister(&mut stream)
                    .unwrap();
            },
            ConnectionStage::MidHandshake(mut handshake) => {
                poll.registry()
                    .deregister(handshake.get_mut().get_mut())
                    .unwrap();
            }
            ConnectionStage::Established(mut peer_connection) => {
                poll.registry()
                    .deregister(peer_connection.ws.get_mut())
                    .unwrap();
            }
        };

        connections.remove(&token);
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
                    stage: ConnectionStage::Stream(stream)
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


pub fn handle_handshake_result(
    connections: &mut ConnectionMap,
    poll_token: Token,
    valid_until: ValidUntil,
    result: Result<WebSocket<TcpStream>, HandshakeError<ServerHandshake<TcpStream, DebugCallback>>> ,
) -> bool {
    match result {
        Ok(mut ws) => {
            println!("handshake established");

            let peer_socket_addr = ws.get_mut().peer_addr().unwrap();

            let peer_connection = PeerConnection {
                ws,
                peer_socket_addr,
                valid_until,
            };

            let connection = Connection {
                valid_until,
                stage: ConnectionStage::Established(peer_connection)
            };

            connections.insert(poll_token, connection);

            false
        },
        Err(HandshakeError::Interrupted(handshake)) => {
            println!("interrupted");

            let connection = Connection {
                valid_until,
                stage: ConnectionStage::MidHandshake(handshake),
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


pub fn run_handshake_and_read_messages(
    socket_worker_index: usize,
    in_message_sender: &InMessageSender,
    poll: &mut Poll,
    connections: &mut ConnectionMap,
    poll_token: Token,
    valid_until: ValidUntil,
){
    println!("poll_token: {}", poll_token.0);

    loop {
        let established = match connections.get(&poll_token).map(|c| &c.stage){
            Some(ConnectionStage::Stream(_)) => false,
            Some(ConnectionStage::MidHandshake(_)) => false,
            Some(ConnectionStage::Established(_)) => true,
            None => break,
        };

        if !established {
            let conn = connections.remove(&poll_token).unwrap();

            match conn.stage {
                ConnectionStage::Stream(stream) => {
                    let handshake_result = ::tungstenite::server::accept_hdr(
                        stream,
                        DebugCallback
                    );

                    let stop_loop = handle_handshake_result(
                        connections,
                        poll_token,
                        valid_until,
                        handshake_result
                    );

                    if stop_loop {
                        break;
                    }
                },
                ConnectionStage::MidHandshake(handshake) => {
                    let stop_loop = handle_handshake_result(
                        connections,
                        poll_token,
                        valid_until,
                        handshake.handshake()
                    );

                    if stop_loop {
                        break;
                    }
                },
                ConnectionStage::Established(_) => unreachable!(),
            }
        } else if let Some(Connection{
            stage: ConnectionStage::Established(peer_connection),
            ..
        }) = connections.get_mut(&poll_token){
            println!("conn established");

            match peer_connection.ws.read_message(){
                Ok(ws_message) => {
                    dbg!(ws_message.clone());

                    if let Some(in_message) = InMessage::from_ws_message(ws_message){
                        dbg!(in_message.clone());

                        let meta = ConnectionMeta {
                            socket_worker_index,
                            socket_worker_poll_token: poll_token,
                            peer_socket_addr: peer_connection.peer_socket_addr
                        };

                        in_message_sender.send((meta, in_message));
                    }

                    peer_connection.valid_until = valid_until;
                },
                Err(tungstenite::Error::Io(err)) => {
                    if err.kind() == ErrorKind::WouldBlock {
                        break
                    }

                    remove_connection_if_exists(poll, connections, poll_token);

                    eprint!("{}", err);

                    break
                },
                Err(tungstenite::Error::ConnectionClosed) => {
                    remove_connection_if_exists(poll, connections, poll_token);

                    break;
                },
                Err(err) => {
                    dbg!(err);

                    remove_connection_if_exists(poll, connections, poll_token);

                    break;
                }
            }
        }
    }
}


pub fn send_out_messages(
    out_message_receiver: ::flume::Drain<(ConnectionMeta, OutMessage)>,
    poll: &mut Poll,
    connections: &mut ConnectionMap,
){
    // Read messages from channel, send to peers
    for (meta, out_message) in out_message_receiver {
        let opt_connection = connections
            .get_mut(&meta.socket_worker_poll_token)
            .map(|v| &mut v.stage);

        if let Some(ConnectionStage::Established(connection)) = opt_connection {
            if connection.peer_socket_addr != meta.peer_socket_addr {
                eprintln!("socket worker: peer socket addrs didn't match");

                continue;
            }

            dbg!(out_message.clone());

            match connection.ws.write_message(out_message.to_ws_message()){
                Ok(()) => {},
                Err(tungstenite::Error::Io(err)) => {
                    if err.kind() == ErrorKind::WouldBlock {
                        continue;
                    }

                    dbg!(err);

                    remove_connection_if_exists(
                        poll,
                        connections,
                        meta.socket_worker_poll_token
                    );
                },
                Err(tungstenite::Error::ConnectionClosed) => {
                    remove_connection_if_exists(
                        poll,
                        connections,
                        meta.socket_worker_poll_token
                    );
                },
                Err(err) => {
                    dbg!(err);
                },
            }
        }
    }
}