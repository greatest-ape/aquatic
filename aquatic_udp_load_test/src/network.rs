use std::io::Cursor;
use std::net::SocketAddr;
use std::sync::atomic::Ordering;
use std::time::Duration;

use crossbeam_channel::{Receiver, Sender};
use mio::{net::UdpSocket, Events, Poll, Interest, Token};
use socket2::{Socket, Domain, Type, Protocol};

use aquatic_udp_protocol::*;

use crate::common::*;


const MAX_PACKET_SIZE: usize = 4096;


pub fn create_socket(
    config: &Config,
    addr: SocketAddr
) -> ::std::net::UdpSocket {
    let socket = if addr.is_ipv4(){
        Socket::new(Domain::ipv4(), Type::dgram(), Some(Protocol::udp()))
    } else {
        Socket::new(Domain::ipv6(), Type::dgram(), Some(Protocol::udp()))
    }.expect("create socket");

    socket.set_nonblocking(true)
        .expect("socket: set nonblocking");
    
    if config.network.recv_buffer != 0 {
        if let Err(err) = socket.set_recv_buffer_size(config.network.recv_buffer){
            eprintln!(
                "socket: failed setting recv buffer to {}: {:?}",
                config.network.recv_buffer,
                err
            );
        }
    }

    socket.bind(&addr.into())
        .unwrap_or_else(|err| panic!("socket: bind to {}: {:?}", addr, err));

    socket.connect(&config.server_address.into())
        .expect("socket: connect to server");

    socket.into_udp_socket()
}


pub fn run_socket_thread(
    state: LoadTestState,
    response_channel_sender: Sender<(ThreadId, Response)>,
    request_receiver: Receiver<Request>,
    config: &Config,
    addr: SocketAddr,
    thread_id: ThreadId
) {
    let mut socket = UdpSocket::from_std(create_socket(config, addr));
    let mut buffer = [0u8; MAX_PACKET_SIZE];

    let token = Token(thread_id.0 as usize);
    let interests = Interest::READABLE;
    let timeout = Duration::from_micros(config.network.poll_timeout);

    let mut poll = Poll::new().expect("create poll");

    poll.registry()
        .register(&mut socket, token, interests)
        .unwrap();

    let mut events = Events::with_capacity(config.network.poll_event_capacity);

    let mut local_state = SocketWorkerLocalStatistics::default();
    let mut responses = Vec::new();

    loop {
        poll.poll(&mut events, Some(timeout))
            .expect("failed polling");

        for event in events.iter(){
            if (event.token() == token) & event.is_readable(){
                read_responses(
                    thread_id,
                    &socket,
                    &mut buffer,
                    &mut local_state,
                    &mut responses
                );

                for r in responses.drain(..){
                    response_channel_sender.send(r)
                        .unwrap_or_else(|err| panic!(
                            "add response to channel in socket worker {}: {:?}",
                            thread_id.0,
                            err
                        ));
                }

                poll.registry()
                    .reregister(&mut socket, token, interests)
                    .unwrap();
            }

            send_requests(
                &state,
                &mut socket,
                &mut buffer,
                &request_receiver,
                &mut local_state
            );
        }

        send_requests(
            &state,
            &mut socket,
            &mut buffer,
            &request_receiver,
            &mut local_state
        );
    }
}


fn read_responses(
    thread_id: ThreadId,
    socket: &UdpSocket,
    buffer: &mut [u8],
    ls: &mut SocketWorkerLocalStatistics,
    responses: &mut Vec<(ThreadId, Response)>,
){
    while let Ok(amt) = socket.recv(buffer) {
        match Response::from_bytes(&buffer[0..amt]){
            Ok(response) => {
                match response {
                    Response::Announce(ref r) => {
                        ls.responses_announce += 1;
                        ls.response_peers += r.peers.len();
                    },
                    Response::Scrape(_) => {
                        ls.responses_scrape += 1;
                    },
                    Response::Connect(_) => {
                        ls.responses_connect += 1;
                    },
                    Response::Error(_) => {
                        ls.responses_error += 1;
                    },
                }

                responses.push((thread_id, response))
            },
            Err(err) => {
                eprintln!("Received invalid response: {:#?}", err);
            }
        }
    }
}


fn send_requests(
    state: &LoadTestState,
    socket: &mut UdpSocket,
    buffer: &mut [u8],
    receiver: &Receiver<Request>,
    statistics: &mut SocketWorkerLocalStatistics,
){
    let mut cursor = Cursor::new(buffer);

    while let Ok(request) = receiver.try_recv() {
        cursor.set_position(0);

        if let Err(err) = request.write(&mut cursor){
            eprintln!("request_to_bytes err: {}", err);
        }

        let position = cursor.position() as usize;
        let inner = cursor.get_ref();

        match socket.send(&inner[..position]) {
            Ok(_) => {
                statistics.requests += 1;
            },
            Err(err) => {
                eprintln!("Couldn't send packet: {:?}", err);
            }
        }
    }
    
    state.statistics.requests
        .fetch_add(statistics.requests, Ordering::SeqCst);
    state.statistics.responses_connect
        .fetch_add(statistics.responses_connect, Ordering::SeqCst);
    state.statistics.responses_announce
        .fetch_add(statistics.responses_announce, Ordering::SeqCst);
    state.statistics.responses_scrape
        .fetch_add(statistics.responses_scrape, Ordering::SeqCst);
    state.statistics.responses_error
        .fetch_add(statistics.responses_error, Ordering::SeqCst);
    state.statistics.response_peers
        .fetch_add(statistics.response_peers, Ordering::SeqCst);

    *statistics = SocketWorkerLocalStatistics::default();
}