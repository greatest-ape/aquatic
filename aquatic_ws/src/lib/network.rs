use std::net::{SocketAddr};
use std::time::{Duration, Instant};
use std::io::ErrorKind;
use std::option::Option;

use slab::Slab;
use tungstenite::{Message, WebSocket};

use mio::{Events, Poll, Interest, Token};
use mio::net::{TcpListener, TcpStream};

use crate::common::*;
use crate::protocol::*;


pub struct PeerConnection {
    pub ws: WebSocket<TcpStream>,
    pub peer_socket_addr: SocketAddr,
    pub valid_until: Instant,
}


pub fn run_socket_worker(
    socket_worker_index: usize,
    in_message_sender: InMessageSender,
    out_message_receiver: OutMessageReceiver,
){
    let address: SocketAddr = "0.0.0.0:3000".parse().unwrap();

    let mut listener = TcpListener::bind(address).unwrap();
    let mut poll = Poll::new().expect("create poll");

    poll.registry()
        .register(&mut listener, Token(0), Interest::READABLE)
        .unwrap();

    let mut events = Events::with_capacity(1024);

    let timeout = Duration::from_millis(50);

    let mut connections: Slab<Option<PeerConnection>> = Slab::new();

    // Insert empty first entry to prevent assignment of index 0
    assert_eq!(connections.insert(None), 0);

    loop {
        poll.poll(&mut events, Some(timeout))
            .expect("failed polling");
        
        let valid_until = Instant::now() + Duration::from_secs(600);

        for event in events.iter(){
            let token = event.token();

            if token.0 == 0 {
                loop {
                    match listener.accept(){
                        Ok((mut stream, src)) => {
                            let entry = connections.vacant_entry();
                            let token = Token(entry.key());

                            poll.registry()
                                .register(&mut stream, token, Interest::READABLE)
                                .unwrap();
                            
                            // FIXME: will this cause issues due to blocking?
                            // Should handshake be started manually below
                            // instead?
                            let ws = tungstenite::server::accept(stream).unwrap();

                            let peer_connection = PeerConnection {
                                ws,
                                peer_socket_addr: src,
                                valid_until,
                            };

                            entry.insert(Some(peer_connection));
                        },
                        Err(err) => {
                            if err.kind() == ErrorKind::WouldBlock {
                                break
                            }

                            eprint!("{}", err);
                        }
                    }
                }
            } else if event.is_readable(){
                loop {
                    if let Some(Some(connection)) = connections.get_mut(token.0){
                        match connection.ws.read_message(){
                            Ok(message) => {
                                // FIXME: parse message, send to handler
                                // through channel (flume?)

                                let meta = MessageMeta {
                                    socket_worker_index,
                                    peer_connection_index: token.0,
                                    peer_socket_addr: connection.peer_socket_addr
                                };

                                let in_message = InMessage::ScrapeRequest(ScrapeRequest {
                                    info_hashes: None,
                                });

                                in_message_sender.send((meta, in_message));

                                connection.valid_until = valid_until;
                            },
                            Err(tungstenite::Error::Io(err)) => {
                                if err.kind() == ErrorKind::WouldBlock {
                                    break
                                }

                                eprint!("{}", err);
                            },
                            Err(tungstenite::Error::ConnectionClosed) => {
                                // FIXME: necessary?
                                poll.registry().deregister(connection.ws.get_mut()).unwrap();

                                connections.remove(token.0);
                            },
                            Err(err) => {
                                eprint!("{}", err);
                            }
                        }
                    }
                }
            }
        }

        let now = Instant::now();

        // Close connections after some time of inactivity and write pending
        // messages (which is required after closing anyway.)
        //
        // FIXME: peers need to be removed too, wherever they are stored
        connections.retain(|_, opt_connection| {
            if let Some(connection) = opt_connection {
                if connection.valid_until < now {
                    connection.ws.close(None).unwrap();
                }

                loop {
                    match connection.ws.write_pending(){
                        Err(tungstenite::Error::Io(err)) => {
                            if err.kind() == ErrorKind::WouldBlock {
                                break
                            }
                        },
                        Err(tungstenite::Error::ConnectionClosed) => {
                            // FIXME: necessary?
                            poll.registry()
                                .deregister(connection.ws.get_mut())
                                .unwrap();

                            return false;
                        },
                        _ => {}
                    }
                }
            }

            true
        });

        // TODO: loop through responses from channel, write them to wss or
        // possibly register ws as writable (but this means event capacity
        // must be adjusted accordingy and is limiting)

        // How should IP's be handled? Send index and src to processing and
        // lookup on return that entry is correct. Old ideas:
        // Maybe use IndexMap<SocketAddr, WebSocket> and use numerical
        // index for token? Removing element from IndexMap requires shifting
        // or swapping indeces, so not very good.

        for (meta, out_message) in out_message_receiver.drain(){
            let message = Message::Text("test".to_string());

            let opt_connection = connections
                .get_mut(meta.peer_connection_index);

            if let Some(Some(connection)) = opt_connection {
                if connection.peer_socket_addr != meta.peer_socket_addr {
                    eprintln!("socket worker: peer socket addrs didn't match");

                    continue;
                }

                match connection.ws.write_message(message){
                    Ok(()) => {},
                    Err(tungstenite::Error::Io(err)) => {
                        if err.kind() == ErrorKind::WouldBlock {
                            continue;
                        }

                        eprint!("{}", err);
                    },
                    Err(tungstenite::Error::ConnectionClosed) => {
                        // FIXME: necessary?
                        poll.registry()
                            .deregister(connection.ws.get_mut())
                            .unwrap();

                        connections.remove(meta.peer_connection_index);
                    },
                    Err(err) => {
                        eprint!("{}", err);
                    },
                }
            }
        }
    }
}