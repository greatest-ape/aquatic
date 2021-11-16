use std::{io::Cursor, vec::Drain};
use std::net::SocketAddr;
use std::sync::atomic::Ordering;
use std::time::Duration;

use mio::{net::UdpSocket, Events, Interest, Poll, Token};
use rand::{SeedableRng, prelude::SmallRng, thread_rng};
use rand_distr::Pareto;
use socket2::{Domain, Protocol, Socket, Type};

use aquatic_udp_protocol::*;

use crate::{common::*, handler::{process_response}, utils::*};

const MAX_PACKET_SIZE: usize = 4096;

pub fn create_socket(config: &Config, addr: SocketAddr) -> ::std::net::UdpSocket {
    let socket = if addr.is_ipv4() {
        Socket::new(Domain::IPV4, Type::DGRAM, Some(Protocol::UDP))
    } else {
        Socket::new(Domain::IPV6, Type::DGRAM, Some(Protocol::UDP))
    }
    .expect("create socket");

    socket
        .set_nonblocking(true)
        .expect("socket: set nonblocking");

    if config.network.recv_buffer != 0 {
        if let Err(err) = socket.set_recv_buffer_size(config.network.recv_buffer) {
            eprintln!(
                "socket: failed setting recv buffer to {}: {:?}",
                config.network.recv_buffer, err
            );
        }
    }

    socket
        .bind(&addr.into())
        .unwrap_or_else(|err| panic!("socket: bind to {}: {:?}", addr, err));

    socket
        .connect(&config.server_address.into())
        .expect("socket: connect to server");

    socket.into()
}

pub fn run_worker_thread(
    state: LoadTestState,
    pareto: Pareto<f64>,
    config: &Config,
    addr: SocketAddr,
    thread_id: ThreadId,
) {
    let mut socket = UdpSocket::from_std(create_socket(config, addr));
    let mut buffer = [0u8; MAX_PACKET_SIZE];

    let mut rng = SmallRng::from_rng(thread_rng()).expect("create SmallRng from thread_rng()");
    let mut torrent_peers = TorrentPeerMap::default();

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
    let mut requests = Vec::new();

    // Bootstrap request cycle by adding a request
    requests.push(create_connect_request(generate_transaction_id(&mut thread_rng())));

    loop {
        poll.poll(&mut events, Some(timeout))
            .expect("failed polling");

        for event in events.iter() {
            if (event.token() == token) & event.is_readable() {
                read_responses(
                    &socket,
                    &mut buffer,
                    &mut local_state,
                    &mut responses,
                );
            }
        }

        let total_responses = responses.len() + if thread_id.0 == 0 {
            state.responses.fetch_and(0, Ordering::SeqCst)
        } else {
            state.responses.fetch_add(responses.len(), Ordering::SeqCst)
        };

        // Somewhat dubious heuristic for deciding how fast to create
        // and send additional requests
        let num_additional_to_send = {
            let n = total_responses as f64 / (config.workers as f64 * 4.0);

            (n * config.handler.additional_request_factor) as usize + 10
        };

        for _ in 0..num_additional_to_send {
            requests.push(create_connect_request(generate_transaction_id(&mut rng)));
        }

        for response in responses.drain(..) {
            let opt_request =
                process_response(&mut rng, pareto, &state.info_hashes, &config, &mut torrent_peers, response);

            if let Some(new_request) = opt_request {
                requests.push(new_request);
            }
        }

        send_requests(
            &state,
            &mut socket,
            &mut buffer,
            &mut local_state,
            requests.drain(..),
        );
    }
}

fn read_responses(
    socket: &UdpSocket,
    buffer: &mut [u8],
    ls: &mut SocketWorkerLocalStatistics,
    responses: &mut Vec<Response>,
) {
    while let Ok(amt) = socket.recv(buffer) {
        match Response::from_bytes(&buffer[0..amt]) {
            Ok(response) => {
                match response {
                    Response::AnnounceIpv4(ref r) => {
                        ls.responses_announce += 1;
                        ls.response_peers += r.peers.len();
                    }
                    Response::AnnounceIpv6(ref r) => {
                        ls.responses_announce += 1;
                        ls.response_peers += r.peers.len();
                    }
                    Response::Scrape(_) => {
                        ls.responses_scrape += 1;
                    }
                    Response::Connect(_) => {
                        ls.responses_connect += 1;
                    }
                    Response::Error(_) => {
                        ls.responses_error += 1;
                    }
                }

                responses.push(response)
            }
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
    statistics: &mut SocketWorkerLocalStatistics,
    requests: Drain<Request>,
) {
    let mut cursor = Cursor::new(buffer);

    for request in requests {
        cursor.set_position(0);

        if let Err(err) = request.write(&mut cursor) {
            eprintln!("request_to_bytes err: {}", err);
        }

        let position = cursor.position() as usize;
        let inner = cursor.get_ref();

        match socket.send(&inner[..position]) {
            Ok(_) => {
                statistics.requests += 1;
            }
            Err(err) => {
                eprintln!("Couldn't send packet: {:?}", err);
            }
        }
    }

    state
        .statistics
        .requests
        .fetch_add(statistics.requests, Ordering::SeqCst);
    state
        .statistics
        .responses_connect
        .fetch_add(statistics.responses_connect, Ordering::SeqCst);
    state
        .statistics
        .responses_announce
        .fetch_add(statistics.responses_announce, Ordering::SeqCst);
    state
        .statistics
        .responses_scrape
        .fetch_add(statistics.responses_scrape, Ordering::SeqCst);
    state
        .statistics
        .responses_error
        .fetch_add(statistics.responses_error, Ordering::SeqCst);
    state
        .statistics
        .response_peers
        .fetch_add(statistics.response_peers, Ordering::SeqCst);

    *statistics = SocketWorkerLocalStatistics::default();
}
