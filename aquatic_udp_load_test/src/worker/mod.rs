mod request_gen;

use std::io::Cursor;
use std::net::SocketAddr;
use std::sync::atomic::Ordering;
use std::time::Duration;

use mio::{net::UdpSocket, Events, Interest, Poll, Token};
use rand::Rng;
use rand::{prelude::SmallRng, thread_rng, SeedableRng};
use rand_distr::Pareto;
use socket2::{Domain, Protocol, Socket, Type};

use aquatic_udp_protocol::*;

use crate::config::Config;
use crate::{common::*, utils::*};
use request_gen::process_response;

const MAX_PACKET_SIZE: usize = 8192;

pub fn run_worker_thread(
    state: LoadTestState,
    pareto: Pareto<f64>,
    config: &Config,
    addr: SocketAddr,
) {
    let mut socket = UdpSocket::from_std(create_socket(config, addr));
    let mut buffer = [0u8; MAX_PACKET_SIZE];

    let mut rng = SmallRng::from_rng(thread_rng()).expect("create SmallRng from thread_rng()");
    let mut torrent_peers = TorrentPeerMap::default();

    let token = Token(0);
    let interests = Interest::READABLE;
    let timeout = Duration::from_micros(config.network.poll_timeout);

    let mut poll = Poll::new().expect("create poll");

    poll.registry()
        .register(&mut socket, token, interests)
        .unwrap();

    let mut events = Events::with_capacity(config.network.poll_event_capacity);

    let mut statistics = SocketWorkerLocalStatistics::default();

    // Bootstrap request cycle
    let initial_request = create_connect_request(generate_transaction_id(&mut thread_rng()));
    send_request(&mut socket, &mut buffer, &mut statistics, initial_request);

    loop {
        poll.poll(&mut events, Some(timeout))
            .expect("failed polling");

        for event in events.iter() {
            if (event.token() == token) & event.is_readable() {
                while let Ok(amt) = socket.recv(&mut buffer) {
                    match Response::from_bytes(&buffer[0..amt], addr.is_ipv4()) {
                        Ok(response) => {
                            match response {
                                Response::AnnounceIpv4(ref r) => {
                                    statistics.responses_announce += 1;
                                    statistics.response_peers += r.peers.len();
                                }
                                Response::AnnounceIpv6(ref r) => {
                                    statistics.responses_announce += 1;
                                    statistics.response_peers += r.peers.len();
                                }
                                Response::Scrape(_) => {
                                    statistics.responses_scrape += 1;
                                }
                                Response::Connect(_) => {
                                    statistics.responses_connect += 1;
                                }
                                Response::Error(_) => {
                                    statistics.responses_error += 1;
                                }
                            }

                            let opt_request = process_response(
                                &mut rng,
                                pareto,
                                &state.info_hashes,
                                &config,
                                &mut torrent_peers,
                                response,
                            );

                            if let Some(request) = opt_request {
                                send_request(&mut socket, &mut buffer, &mut statistics, request);
                            }
                        }
                        Err(err) => {
                            eprintln!("Received invalid response: {:#?}", err);
                        }
                    }
                }

                if rng.gen::<f32>() <= config.requests.additional_request_probability {
                    let additional_request =
                        create_connect_request(generate_transaction_id(&mut rng));

                    send_request(
                        &mut socket,
                        &mut buffer,
                        &mut statistics,
                        additional_request,
                    );
                }

                update_shared_statistics(&state, &mut statistics);
            }
        }
    }
}

fn send_request(
    socket: &mut UdpSocket,
    buffer: &mut [u8],
    statistics: &mut SocketWorkerLocalStatistics,
    request: Request,
) {
    let mut cursor = Cursor::new(buffer);

    match request.write(&mut cursor) {
        Ok(()) => {
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
        Err(err) => {
            eprintln!("request_to_bytes err: {}", err);
        }
    }
}

fn update_shared_statistics(state: &LoadTestState, statistics: &mut SocketWorkerLocalStatistics) {
    state
        .statistics
        .requests
        .fetch_add(statistics.requests, Ordering::Relaxed);
    state
        .statistics
        .responses_connect
        .fetch_add(statistics.responses_connect, Ordering::Relaxed);
    state
        .statistics
        .responses_announce
        .fetch_add(statistics.responses_announce, Ordering::Relaxed);
    state
        .statistics
        .responses_scrape
        .fetch_add(statistics.responses_scrape, Ordering::Relaxed);
    state
        .statistics
        .responses_error
        .fetch_add(statistics.responses_error, Ordering::Relaxed);
    state
        .statistics
        .response_peers
        .fetch_add(statistics.response_peers, Ordering::Relaxed);

    *statistics = SocketWorkerLocalStatistics::default();
}

fn create_socket(config: &Config, addr: SocketAddr) -> ::std::net::UdpSocket {
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
