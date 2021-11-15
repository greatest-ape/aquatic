use std::io::{Cursor, ErrorKind};
use std::net::{Ipv4Addr, SocketAddr, SocketAddrV4, SocketAddrV6};
use std::sync::{
    atomic::{AtomicUsize, Ordering},
    Arc,
};
use std::time::{Duration, Instant};
use std::vec::Drain;

use aquatic_common::access_list::create_access_list_cache;
use aquatic_common::ValidUntil;
use crossbeam_channel::{Receiver, Sender};
use mio::net::UdpSocket;
use mio::{Events, Interest, Poll, Token};
use rand::prelude::{SeedableRng, StdRng};

use aquatic_udp_protocol::{Request, Response};

use crate::common::network::*;
use crate::common::*;
use crate::config::Config;

pub fn run_socket_worker(
    state: State,
    config: Config,
    token_num: usize,
    request_sender: Sender<(ConnectedRequest, SocketAddr)>,
    response_receiver: Receiver<(ConnectedResponse, SocketAddr)>,
    num_bound_sockets: Arc<AtomicUsize>,
) {
    let mut rng = StdRng::from_entropy();
    let mut buffer = [0u8; MAX_PACKET_SIZE];

    let mut socket = UdpSocket::from_std(create_socket(&config));
    let mut poll = Poll::new().expect("create poll");

    let interests = Interest::READABLE;

    poll.registry()
        .register(&mut socket, Token(token_num), interests)
        .unwrap();

    num_bound_sockets.fetch_add(1, Ordering::SeqCst);

    let mut events = Events::with_capacity(config.network.poll_event_capacity);
    let mut connections = ConnectionMap::default();

    let mut local_responses: Vec<(Response, SocketAddr)> = Vec::new();

    let timeout = Duration::from_millis(50);

    let cleaning_duration = Duration::from_secs(config.cleaning.connection_cleaning_interval);

    let mut iter_counter = 0usize;
    let mut last_cleaning = Instant::now();

    loop {
        poll.poll(&mut events, Some(timeout))
            .expect("failed polling");

        for event in events.iter() {
            let token = event.token();

            if (token.0 == token_num) & event.is_readable() {
                read_requests(
                    &config,
                    &state,
                    &mut connections,
                    &mut rng,
                    &mut socket,
                    &mut buffer,
                    &request_sender,
                    &mut local_responses,
                );
            }
        }

        send_responses(
            &state,
            &config,
            &mut socket,
            &mut buffer,
            &response_receiver,
            local_responses.drain(..),
        );

        if iter_counter % 32 == 0 {
            let now = Instant::now();

            if now > last_cleaning + cleaning_duration {
                connections.clean();

                last_cleaning = now;
            }
        }

        iter_counter = iter_counter.wrapping_add(1);
    }
}

#[inline]
fn read_requests(
    config: &Config,
    state: &State,
    connections: &mut ConnectionMap,
    rng: &mut StdRng,
    socket: &mut UdpSocket,
    buffer: &mut [u8],
    request_sender: &Sender<(ConnectedRequest, SocketAddr)>,
    local_responses: &mut Vec<(Response, SocketAddr)>,
) {
    let mut requests_received: usize = 0;
    let mut bytes_received: usize = 0;

    let valid_until = ValidUntil::new(config.cleaning.max_connection_age);

    let mut access_list_cache = create_access_list_cache(&state.access_list);

    loop {
        match socket.recv_from(&mut buffer[..]) {
            Ok((amt, src)) => {
                let res_request =
                    Request::from_bytes(&buffer[..amt], config.protocol.max_scrape_torrents);

                bytes_received += amt;

                if res_request.is_ok() {
                    requests_received += 1;
                }

                let src = match src {
                    SocketAddr::V6(src) => {
                        match src.ip().octets() {
                            // Convert IPv4-mapped address (available in std but nightly-only)
                            [0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0xff, 0xff, a, b, c, d] => {
                                SocketAddr::V4(SocketAddrV4::new(
                                    Ipv4Addr::new(a, b, c, d),
                                    src.port(),
                                ))
                            }
                            _ => src.into(),
                        }
                    }
                    src => src,
                };

                handle_request(
                    config,
                    connections,
                    &mut access_list_cache,
                    rng,
                    request_sender,
                    local_responses,
                    valid_until,
                    res_request,
                    src,
                );
            }
            Err(err) => {
                if err.kind() == ErrorKind::WouldBlock {
                    break;
                }

                ::log::info!("recv_from error: {}", err);
            }
        }
    }

    if config.statistics.interval != 0 {
        state
            .statistics
            .requests_received
            .fetch_add(requests_received, Ordering::SeqCst);
        state
            .statistics
            .bytes_received
            .fetch_add(bytes_received, Ordering::SeqCst);
    }
}

#[inline]
fn send_responses(
    state: &State,
    config: &Config,
    socket: &mut UdpSocket,
    buffer: &mut [u8],
    response_receiver: &Receiver<(ConnectedResponse, SocketAddr)>,
    local_responses: Drain<(Response, SocketAddr)>,
) {
    let mut responses_sent: usize = 0;
    let mut bytes_sent: usize = 0;

    let mut cursor = Cursor::new(buffer);

    let response_iterator = local_responses.into_iter().chain(
        response_receiver
            .try_iter()
            .map(|(response, addr)| (response.into(), addr)),
    );

    for (response, addr) in response_iterator {
        cursor.set_position(0);

        let addr = if config.network.address.is_ipv4() {
            if let SocketAddr::V4(addr) = addr {
                SocketAddr::V4(addr)
            } else {
                unreachable!()
            }
        } else {
            match addr {
                SocketAddr::V4(addr) => {
                    let ip = addr.ip().to_ipv6_mapped();

                    SocketAddr::V6(SocketAddrV6::new(ip, addr.port(), 0, 0))
                }
                addr => addr,
            }
        };

        match response.write(&mut cursor) {
            Ok(()) => {
                let amt = cursor.position() as usize;

                match socket.send_to(&cursor.get_ref()[..amt], addr) {
                    Ok(amt) => {
                        responses_sent += 1;
                        bytes_sent += amt;
                    }
                    Err(err) => {
                        if err.kind() == ErrorKind::WouldBlock {
                            break;
                        }

                        ::log::info!("send_to error: {}", err);
                    }
                }
            }
            Err(err) => {
                ::log::error!("Response::write error: {:?}", err);
            }
        }
    }

    if config.statistics.interval != 0 {
        state
            .statistics
            .responses_sent
            .fetch_add(responses_sent, Ordering::SeqCst);
        state
            .statistics
            .bytes_sent
            .fetch_add(bytes_sent, Ordering::SeqCst);
    }
}
