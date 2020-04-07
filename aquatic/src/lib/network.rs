use std::sync::atomic::Ordering;
use std::net::SocketAddr;
use std::io::ErrorKind;

use mio::{Events, Poll, Interest, Token};
use mio::net::UdpSocket;
use net2::{UdpSocketExt, UdpBuilder};
use net2::unix::UnixUdpBuilderExt;

use bittorrent_udp::types::IpVersion;
use bittorrent_udp::converters::{response_to_bytes, request_from_bytes};

use crate::common::*;
use crate::config::Config;
use crate::handlers::*;


pub fn run_event_loop(
    state: State,
    config: Config,
    token_num: usize,
){
    let mut buffer = [0u8; MAX_PACKET_SIZE];

    let mut socket = UdpSocket::from_std(create_socket(&config));
    let mut poll = Poll::new().expect("create poll");

    let interests = Interest::READABLE;

    poll.registry()
        .register(&mut socket, Token(token_num), interests)
        .unwrap();

    let mut events = Events::with_capacity(config.poll_event_capacity);

    let mut connect_requests: Vec<(ConnectRequest, SocketAddr)> = Vec::with_capacity(1024);
    let mut announce_requests: Vec<(AnnounceRequest, SocketAddr)> = Vec::with_capacity(1024);
    let mut scrape_requests: Vec<(ScrapeRequest, SocketAddr)> = Vec::with_capacity(1024);
    let mut responses: Vec<(Response, SocketAddr)> = Vec::with_capacity(1024);

    loop {
        poll.poll(&mut events, None)
            .expect("failed polling");

        for event in events.iter(){
            let token = event.token();

            if token.0 == token_num {
                if event.is_readable(){
                    handle_readable_socket(
                        &state,
                        &config,
                        &mut socket,
                        &mut buffer,
                        &mut responses,
                        &mut connect_requests,
                        &mut announce_requests,
                        &mut scrape_requests
                    );

                    poll.registry()
                        .reregister(&mut socket, token, interests)
                        .unwrap();
                }
            }
        }
    }
}


fn create_socket(config: &Config) -> ::std::net::UdpSocket {
    let mut builder = &{
        if config.address.is_ipv4(){
            UdpBuilder::new_v4().expect("socket: build")
        } else {
            UdpBuilder::new_v6().expect("socket: build")
        }
    };

    builder = builder.reuse_port(true)
        .expect("socket: set reuse port");

    let socket = builder.bind(&config.address)
        .expect(&format!("socket: bind to {}", &config.address));

    socket.set_nonblocking(true)
        .expect("socket: set nonblocking");
    
    if let Err(err) = socket.set_recv_buffer_size(config.recv_buffer_size){
        eprintln!(
            "socket: failed setting recv buffer to {}: {:?}",
            config.recv_buffer_size,
            err
        );
    }

    socket
}


/// Read requests, generate and send back responses
fn handle_readable_socket(
    state: &State,
    config: &Config,
    socket: &mut UdpSocket,
    buffer: &mut [u8],
    responses: &mut Vec<(Response, SocketAddr)>,
    connect_requests: &mut Vec<(ConnectRequest, SocketAddr)>,
    announce_requests: &mut Vec<(AnnounceRequest, SocketAddr)>,
    scrape_requests: &mut Vec<(ScrapeRequest, SocketAddr)>,
){
    let mut requests_received: usize = 0;
    let mut responses_sent: usize = 0;

    loop {
        match socket.recv_from(buffer) {
            Ok((amt, src)) => {
                let request = request_from_bytes(
                    &buffer[..amt],
                    config.max_scrape_torrents
                );

                if request.is_ok(){
                    requests_received += 1;
                }

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
                        eprintln!("request_from_bytes error: {:?}", err);
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
        state,
        responses,
        connect_requests.drain(..)
    );
    handle_announce_requests(
        state,
        config,
        responses,
        announce_requests.drain(..),
    );
    handle_scrape_requests(
        state,
        responses,
        scrape_requests.drain(..),
    );

    for (response, src) in responses.drain(..) {
        let bytes = response_to_bytes(&response, IpVersion::IPv4);

        match socket.send_to(&bytes[..], src){
            Ok(_bytes_sent) => {
                responses_sent += 1;
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

    state.statistics.requests_received
        .fetch_add(requests_received, Ordering::SeqCst);
    state.statistics.responses_sent
        .fetch_add(responses_sent, Ordering::SeqCst);
}