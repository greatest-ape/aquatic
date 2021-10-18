use std::io::{Cursor, ErrorKind};
use std::net::{IpAddr, SocketAddr};
use std::sync::{
    atomic::{AtomicUsize, Ordering},
    Arc,
};
use std::time::{Duration, Instant};
use std::vec::Drain;

use crossbeam_channel::{Receiver, Sender};
use hashbrown::HashMap;
use mio::net::UdpSocket;
use mio::{Events, Interest, Poll, Token};
use rand::prelude::{Rng, SeedableRng, StdRng};
use socket2::{Domain, Protocol, Socket, Type};

use aquatic_udp_protocol::{IpVersion, Request, Response};

use crate::common::*;
use crate::config::Config;

#[derive(Default)]
struct ConnectionMap(HashMap<(ConnectionId, SocketAddr), ValidUntil>);

impl ConnectionMap {
    fn insert(
        &mut self,
        connection_id: ConnectionId,
        socket_addr: SocketAddr,
        valid_until: ValidUntil,
    ) {
        self.0.insert((connection_id, socket_addr), valid_until);
    }

    fn contains(&mut self, connection_id: ConnectionId, socket_addr: SocketAddr) -> bool {
        self.0.contains_key(&(connection_id, socket_addr))
    }

    fn clean(&mut self) {
        let now = Instant::now();

        self.0.retain(|_, v| v.0 > now);
        self.0.shrink_to_fit();
    }
}

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

    let mut iter_counter = 0usize;

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

                state
                    .statistics
                    .readable_events
                    .fetch_add(1, Ordering::SeqCst);
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

        iter_counter += 1;

        if iter_counter == 1000 {
            connections.clean();

            iter_counter = 0;
        }
    }
}

fn create_socket(config: &Config) -> ::std::net::UdpSocket {
    let socket = if config.network.address.is_ipv4() {
        Socket::new(Domain::IPV4, Type::DGRAM, Some(Protocol::UDP))
    } else {
        Socket::new(Domain::IPV6, Type::DGRAM, Some(Protocol::UDP))
    }
    .expect("create socket");

    socket.set_reuse_port(true).expect("socket: set reuse port");

    socket
        .set_nonblocking(true)
        .expect("socket: set nonblocking");

    socket
        .bind(&config.network.address.into())
        .unwrap_or_else(|err| panic!("socket: bind to {}: {:?}", config.network.address, err));

    let recv_buffer_size = config.network.socket_recv_buffer_size;

    if recv_buffer_size != 0 {
        if let Err(err) = socket.set_recv_buffer_size(recv_buffer_size) {
            ::log::error!(
                "socket: failed setting recv buffer to {}: {:?}",
                recv_buffer_size,
                err
            );
        }
    }

    socket.into()
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
    let access_list_mode = config.access_list.mode;

    loop {
        match socket.recv_from(&mut buffer[..]) {
            Ok((amt, src)) => {
                let request =
                    Request::from_bytes(&buffer[..amt], config.protocol.max_scrape_torrents);

                bytes_received += amt;

                if request.is_ok() {
                    requests_received += 1;
                }

                match request {
                    Ok(Request::Connect(request)) => {
                        let connection_id = ConnectionId(rng.gen());

                        connections.insert(connection_id, src, valid_until);

                        let response = Response::Connect(ConnectResponse {
                            connection_id,
                            transaction_id: request.transaction_id,
                        });

                        local_responses.push((response, src))
                    }
                    Ok(Request::Announce(request)) => {
                        if connections.contains(request.connection_id, src) {
                            if state
                                .access_list
                                .allows(access_list_mode, &request.info_hash.0)
                            {
                                if let Err(err) =
                                    request_sender.send((ConnectedRequest::Announce(request), src))
                                {
                                    ::log::warn!("request_sender.send failed: {:?}", err)
                                }
                            } else {
                                let response = Response::Error(ErrorResponse {
                                    transaction_id: request.transaction_id,
                                    message: "Info hash not allowed".into(),
                                });

                                local_responses.push((response, src))
                            }
                        }
                    }
                    Ok(Request::Scrape(request)) => {
                        if connections.contains(request.connection_id, src) {
                            if let Err(err) =
                                request_sender.send((ConnectedRequest::Scrape(request), src))
                            {
                                ::log::warn!("request_sender.send failed: {:?}", err)
                            }
                        }
                    }
                    Err(err) => {
                        ::log::debug!("Request::from_bytes error: {:?}", err);

                        if let RequestParseError::Sendable {
                            connection_id,
                            transaction_id,
                            err,
                        } = err
                        {
                            if connections.contains(connection_id, src) {
                                let response = ErrorResponse {
                                    transaction_id,
                                    message: err.right_or("Parse error").into(),
                                };

                                local_responses.push((response.into(), src));
                            }
                        }
                    }
                }
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

    for (response, src) in response_iterator {
        cursor.set_position(0);

        let ip_version = ip_version_from_ip(src.ip());

        match response.write(&mut cursor, ip_version) {
            Ok(()) => {
                let amt = cursor.position() as usize;

                match socket.send_to(&cursor.get_ref()[..amt], src) {
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

fn ip_version_from_ip(ip: IpAddr) -> IpVersion {
    match ip {
        IpAddr::V4(_) => IpVersion::IPv4,
        IpAddr::V6(ip) => {
            if let [0, 0, 0, 0, 0, 0xffff, ..] = ip.segments() {
                IpVersion::IPv4
            } else {
                IpVersion::IPv6
            }
        }
    }
}
