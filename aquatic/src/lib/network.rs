use std::sync::atomic::Ordering;
use std::io::{Cursor, ErrorKind};
use std::net::SocketAddr;
use std::time::Duration;

use crossbeam_channel::{Sender, Receiver};
use mio::{Events, Poll, Interest, Token};
use mio::net::UdpSocket;
use net2::{UdpSocketExt, UdpBuilder};
use net2::unix::UnixUdpBuilderExt;

use bittorrent_udp::types::IpVersion;
use bittorrent_udp::converters::{response_to_bytes, request_from_bytes};

use crate::common::*;
use crate::config::Config;


pub fn run_socket_worker(
    state: State,
    config: Config,
    token_num: usize,
    request_sender: Sender<(Request, SocketAddr)>,
    response_receiver: Receiver<(Response, SocketAddr)>,
){
    let mut buffer = [0u8; MAX_PACKET_SIZE];

    let mut socket = UdpSocket::from_std(create_socket(&config));
    let mut poll = Poll::new().expect("create poll");

    let interests = Interest::READABLE;

    poll.registry()
        .register(&mut socket, Token(token_num), interests)
        .unwrap();

    let mut events = Events::with_capacity(config.network.poll_event_capacity);

    let timeout = Duration::from_millis(50);

    loop {
        poll.poll(&mut events, Some(timeout))
            .expect("failed polling");

        for event in events.iter(){
            let token = event.token();

            if token.0 == token_num {
                if event.is_readable(){
                    read_requests(
                        &state,
                        &config,
                        &mut socket,
                        &mut buffer,
                        &request_sender,
                    );

                    state.statistics.readable_events.fetch_add(1, Ordering::SeqCst);

                    poll.registry()
                        .reregister(&mut socket, token, interests)
                        .unwrap();
                }
            }
        }

        send_responses(
            &state,
            &config,
            &mut socket,
            &mut buffer,
            &response_receiver,
        );
    }
}


fn create_socket(config: &Config) -> ::std::net::UdpSocket {
    let mut builder = &{
        if config.network.address.is_ipv4(){
            UdpBuilder::new_v4().expect("socket: build")
        } else {
            UdpBuilder::new_v6().expect("socket: build")
        }
    };

    builder = builder.reuse_port(true)
        .expect("socket: set reuse port");

    let socket = builder.bind(&config.network.address)
        .expect(&format!("socket: bind to {}", &config.network.address));

    socket.set_nonblocking(true)
        .expect("socket: set nonblocking");
    
    let recv_buffer_size = config.network.socket_recv_buffer_size;
    
    if recv_buffer_size != 0 {
        if let Err(err) = socket.set_recv_buffer_size(recv_buffer_size){
            eprintln!(
                "socket: failed setting recv buffer to {}: {:?}",
                recv_buffer_size,
                err
            );
        }
    }

    socket
}


#[inline]
fn read_requests(
    state: &State,
    config: &Config,
    socket: &mut UdpSocket,
    buffer: &mut [u8],
    request_sender: &Sender<(Request, SocketAddr)>,
){
    let mut requests_received: usize = 0;
    let mut bytes_received: usize = 0;

    loop {
        match socket.recv_from(&mut buffer[..]) {
            Ok((amt, src)) => {
                let request = request_from_bytes(
                    &buffer[..amt],
                    config.network.max_scrape_torrents
                );

                bytes_received += amt;

                if request.is_ok(){
                    requests_received += 1;
                }

                match request {
                    Ok(request) => {
                        request_sender.try_send((request, src));
                    },
                    Err(err) => {
                        eprintln!("request_from_bytes error: {:?}", err);

                        if let Some(transaction_id) = err.transaction_id {
                            let opt_message = if err.error.is_some() {
                                Some("Parse error".to_string())
                            } else if let Some(message) = err.message {
                                Some(message)
                            } else {
                                None
                            };

                            if let Some(message) = opt_message {
                                let response = ErrorResponse {
                                    transaction_id,
                                    message,
                                };

                                // responses.push((response.into(), src)); // FIXME
                            }
                        }
                    },
                }
            },
            Err(err) => {
                if err.kind() == ErrorKind::WouldBlock {
                    break;
                }

                eprintln!("recv_from error: {}", err);
            }
        }
    }

    if config.statistics.interval != 0 {
        state.statistics.requests_received
            .fetch_add(requests_received, Ordering::SeqCst);
        state.statistics.bytes_received
            .fetch_add(bytes_received, Ordering::SeqCst);
    }
}


#[inline]
fn send_responses(
    state: &State,
    config: &Config,
    socket: &mut UdpSocket,
    buffer: &mut [u8],
    response_receiver: &Receiver<(Response, SocketAddr)>,
){
    let mut responses_sent: usize = 0;
    let mut bytes_sent: usize = 0;

    let mut cursor = Cursor::new(buffer);

    for (response, src) in response_receiver.try_iter(){
        cursor.set_position(0);

        response_to_bytes(&mut cursor, response, IpVersion::IPv4).unwrap();

        let amt = cursor.position() as usize;

        match socket.send_to(&cursor.get_ref()[..amt], src){
            Ok(amt) => {
                responses_sent += 1;
                bytes_sent += amt;
            },
            Err(err) => {
                if err.kind() == ErrorKind::WouldBlock {
                    break;
                }

                eprintln!("send_to error: {}", err);
            }
        }
    }

    if config.statistics.interval != 0 {
        state.statistics.responses_sent
            .fetch_add(responses_sent, Ordering::SeqCst);
        state.statistics.bytes_sent
            .fetch_add(bytes_sent, Ordering::SeqCst);
    }
}