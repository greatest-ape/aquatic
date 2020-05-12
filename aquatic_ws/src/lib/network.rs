use std::fs::File;
use std::io::Read;
use std::net::{SocketAddr};
use std::time::{Duration, Instant};
use std::io::ErrorKind;

use tungstenite::WebSocket;
use tungstenite::handshake::{MidHandshake, HandshakeError, server::{ServerHandshake, NoCallback}};
use hashbrown::HashMap;
use native_tls::{Identity, TlsAcceptor, TlsStream};
use net2::{TcpBuilder, unix::UnixTcpBuilderExt};

use mio::{Events, Poll, Interest, Token};
use mio::net::{TcpListener, TcpStream};

use crate::common::*;
use crate::config::Config;
use crate::protocol::*;


pub type Stream = TlsStream<TcpStream>;


pub struct PeerConnection {
    pub ws: WebSocket<Stream>,
    pub peer_addr: SocketAddr,
    pub valid_until: ValidUntil,
}


pub enum ConnectionStage {
    TcpStream(TcpStream),
    TlsMidHandshake(native_tls::MidHandshakeTlsStream<TcpStream>),
    TlsStream(Stream),
    WsHandshake(MidHandshake<ServerHandshake<Stream, DebugCallback>>),
    Established(PeerConnection),
}


pub struct Connection {
    valid_until: ValidUntil,
    stage: ConnectionStage,
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


fn close_and_deregister_connection(
    poll: &mut Poll,
    connection: &mut Connection,
){
    match connection.stage {
        ConnectionStage::TcpStream(ref mut stream) => {
            poll.registry()
                .deregister(stream)
                .unwrap();
        },
        ConnectionStage::TlsMidHandshake(ref mut handshake) => {
            poll.registry()
                .deregister(handshake.get_mut())
                .unwrap();
        },
        ConnectionStage::TlsStream(ref mut stream) => {
            poll.registry()
                .deregister(stream.get_mut())
                .unwrap();
        },
        ConnectionStage::WsHandshake(ref mut handshake) => {
            poll.registry()
                .deregister(handshake.get_mut().get_mut().get_mut())
                .unwrap();
        },
        ConnectionStage::Established(ref mut peer_connection) => {
            if peer_connection.ws.can_read(){
                peer_connection.ws.close(None).unwrap();

                // Needs to be done after ws.close()
                if let Err(err) = peer_connection.ws.write_pending(){
                    dbg!(err);
                }
            }

            poll.registry()
                .deregister(peer_connection.ws.get_mut().get_mut())
                .unwrap();
        },
    }
}


fn remove_connection_if_exists(
    poll: &mut Poll,
    connections: &mut ConnectionMap,
    token: Token,
){
    if let Some(mut connection) = connections.remove(&token){
        close_and_deregister_connection(poll, &mut connection);

        connections.remove(&token);
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
            close_and_deregister_connection(poll, connection);

            println!("closing connection, it is inactive");

            false
        } else {
            println!("keeping connection, it is still active");

            true
        }
    });

    connections.shrink_to_fit();
}


fn create_listener(config: &Config) -> ::std::net::TcpListener {
    let mut builder = &{
        if config.network.address.is_ipv4(){
            TcpBuilder::new_v4().expect("socket: build")
        } else {
            TcpBuilder::new_v6().expect("socket: build")
        }
    };

    builder = builder.reuse_port(true)
        .expect("socket: set reuse port");

    builder = builder.bind(&config.network.address)
        .expect(&format!("socket: bind to {}", &config.network.address));

    let listener = builder.listen(128)
        .expect("tcpbuilder to tcp listener");

    listener.set_nonblocking(true)
        .expect("socket: set nonblocking");

    listener
}


fn create_tls_acceptor(
    config: &Config,
) -> TlsAcceptor {
    let mut identity_bytes = Vec::new();
    let mut file = File::open(&config.network.pkcs12_path)
        .expect("open pkcs12 file");

    file.read_to_end(&mut identity_bytes).expect("read pkcs12 file");

    let identity = Identity::from_pkcs12(
        &mut identity_bytes,
        &config.network.pkcs12_password
    ).expect("create pkcs12 identity");

    let acceptor = TlsAcceptor::new(identity)   
        .expect("create TlsAcceptor");

    acceptor
}


pub fn run_socket_worker(
    config: Config,
    socket_worker_index: usize,
    in_message_sender: InMessageSender,
    out_message_receiver: OutMessageReceiver,
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

    let tls_acceptor = create_tls_acceptor(&config);

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
                    &mut poll_token_counter
                );
            } else if event.is_readable(){
                run_handshakes_and_read_messages(
                    socket_worker_index,
                    &in_message_sender,
                    &tls_acceptor,
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


pub fn handle_ws_handshake_result(
    connections: &mut ConnectionMap,
    poll_token: Token,
    valid_until: ValidUntil,
    result: Result<WebSocket<Stream>, HandshakeError<ServerHandshake<Stream, DebugCallback>>> ,
) -> bool {
    match result {
        Ok(mut ws) => {
            println!("handshake established");

            let peer_addr = ws.get_mut().get_mut().peer_addr().unwrap();

            let peer_connection = PeerConnection {
                ws,
                peer_addr,
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
                stage: ConnectionStage::WsHandshake(handshake),
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


pub fn run_handshakes_and_read_messages(
    socket_worker_index: usize,
    in_message_sender: &InMessageSender,
    tls_acceptor: &TlsAcceptor,
    poll: &mut Poll,
    connections: &mut ConnectionMap,
    poll_token: Token,
    valid_until: ValidUntil,
){
    println!("poll_token: {}", poll_token.0);

    loop {
        let established = match connections.get(&poll_token).map(|c| &c.stage){
            Some(ConnectionStage::Established(_)) => true,
            Some(_) => false,
            None => break,
        };

        if !established {
            let conn = connections.remove(&poll_token).unwrap();

            match conn.stage {
                ConnectionStage::TcpStream(stream) => {
                    let stop_loop = handle_tls_handshake_result(
                        connections,
                        poll_token,
                        valid_until,
                        tls_acceptor.accept(stream)
                    );

                    if stop_loop {
                        break;
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

                    let stop_loop = handle_ws_handshake_result(
                        connections,
                        poll_token,
                        valid_until,
                        handshake_result
                    );

                    if stop_loop {
                        break;
                    }
                },
                ConnectionStage::WsHandshake(handshake) => {
                    let stop_loop = handle_ws_handshake_result(
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
                            worker_index: socket_worker_index,
                            poll_token,
                            peer_addr: peer_connection.peer_addr
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


/// Read messages from channel, send to peers
pub fn send_out_messages(
    out_message_receiver: ::flume::Drain<(ConnectionMeta, OutMessage)>,
    poll: &mut Poll,
    connections: &mut ConnectionMap,
){
    for (meta, out_message) in out_message_receiver {
        let opt_connection = connections
            .get_mut(&meta.poll_token)
            .map(|v| &mut v.stage);

        if let Some(ConnectionStage::Established(connection)) = opt_connection {
            if connection.peer_addr != meta.peer_addr {
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
                        meta.poll_token
                    );
                },
                Err(tungstenite::Error::ConnectionClosed) => {
                    remove_connection_if_exists(
                        poll,
                        connections,
                        meta.poll_token
                    );
                },
                Err(err) => {
                    dbg!(err);
                },
            }
        }
    }
}