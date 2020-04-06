use std::net::SocketAddr;
use std::time::Duration;
use std::io::ErrorKind;

use mio::{Events, Poll, Interest, Token};
use mio::net::UdpSocket;
use net2::{UdpSocketExt, UdpBuilder};
use net2::unix::UnixUdpBuilderExt;

use bittorrent_udp::types::IpVersion;
use bittorrent_udp::converters::{response_to_bytes, request_from_bytes};

use crate::common::*;
use crate::handlers::*;


pub fn create_socket(
    addr: SocketAddr,
    recv_buffer_size: usize,
) -> ::std::net::UdpSocket {

    let mut builder = &{
        if addr.is_ipv4(){
            UdpBuilder::new_v4().expect("socket: build")
        } else {
            UdpBuilder::new_v6().expect("socket: build")
        }
    };

    builder = builder.reuse_port(true)
        .expect("socket: set reuse port");

    let socket = builder.bind(&addr)
        .expect(&format!("socket: bind to {}", addr));

    socket.set_nonblocking(true)
        .expect("socket: set nonblocking");
    
    if let Err(err) = socket.set_recv_buffer_size(recv_buffer_size){
        eprintln!(
            "socket: failed setting recv buffer to {}: {:?}",
            recv_buffer_size,
            err
        );
    }

    socket
}


pub fn run_event_loop(
    state: State,
    socket: ::std::net::UdpSocket,
    token_num: usize,
    poll_timeout: Duration,
){
    let mut buffer = [0u8; MAX_PACKET_SIZE];

    let mut socket = UdpSocket::from_std(socket);
    let mut poll = Poll::new().expect("create poll");

    let interests = Interest::READABLE;

    poll.registry()
        .register(&mut socket, Token(token_num), interests)
        .unwrap();

    let mut events = Events::with_capacity(EVENT_CAPACITY);

    let mut connect_requests: Vec<(ConnectRequest, SocketAddr)> = Vec::with_capacity(1024);
    let mut announce_requests: Vec<(AnnounceRequest, SocketAddr)> = Vec::with_capacity(1024);
    let mut scrape_requests: Vec<(ScrapeRequest, SocketAddr)> = Vec::with_capacity(1024);
    let mut responses: Vec<(Response, SocketAddr)> = Vec::with_capacity(1024);

    loop {
        poll.poll(&mut events, Some(poll_timeout))
            .expect("failed polling");

        for event in events.iter(){
            let token = event.token();

            if token.0 == token_num {
                if event.is_readable(){
                    loop {
                        match socket.recv_from(&mut buffer) {
                            Ok((amt, src)) => {
                                let request = request_from_bytes(
                                    &buffer[..amt],
                                    255u8 // FIXME
                                );

                                match request {
                                    Ok(Request::Connect(r)) => {
                                        connect_requests.push((r, src)); 
                                    },
                                    Ok(Request::Announce(r)) => {
                                        announce_requests.push((r, src));
                                    },
                                    Ok(Request::Scrape(r)) => {
                                        scrape_requests.push((r, src));
                                    },
                                    Ok(Request::Invalid(r)) => {
                                        let response = Response::Error(ErrorResponse {
                                            transaction_id: r.transaction_id,
                                            message: "Invalid request".to_string(),
                                        });

                                        responses.push((response, src));
                                    },
                                    Err(err) => {
                                        eprintln!("Request parse error: {:?}", err);
                                    },
                                }
                            },
                            Err(err) => {
                                match err.kind() {
                                    ErrorKind::WouldBlock => {
                                        break;
                                    },
                                    err => {
                                        eprintln!("recv_from error: {:?}", err);
                                    }
                                }
                            }
                        }
                    }

                    handle_connect_requests(
                        &state,
                        &mut responses,
                        connect_requests.drain(..)
                    );
                    handle_announce_requests(
                        &state,
                        &mut responses,
                        announce_requests.drain(..),
                    );
                    handle_scrape_requests(
                        &state,
                        &mut responses,
                        scrape_requests.drain(..),
                    );

                    for (response, src) in responses.drain(..) {
                        let bytes = response_to_bytes(&response, IpVersion::IPv4);

                        match socket.send_to(&bytes[..], src){
                            Ok(_bytes_sent) => {
                            },
                            Err(err) => {
                                match err.kind(){
                                    ErrorKind::WouldBlock => {
                                        break;
                                    },
                                    err => {
                                        eprintln!("send_to error: {:?}", err);
                                    }
                                }
                            }
                        }
                    }

                    poll.registry().reregister(&mut socket, token, interests).unwrap();
                }
            }
        }
    }
}