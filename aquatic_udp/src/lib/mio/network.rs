use std::io::{ErrorKind, IoSliceMut};
use std::net::{IpAddr, SocketAddr};
use std::sync::{
    atomic::{AtomicUsize, Ordering},
    Arc,
};
use std::time::Duration;
use std::vec::Drain;

use aquatic_common::access_list::{AccessListArcSwap, AccessListMode, AccessListQuery};
use crossbeam_channel::{Receiver, Sender};
use mio::net::UdpSocket;
use mio::{Events, Interest, Poll, Token};
use rand::prelude::{Rng, SeedableRng, StdRng};
use socket2::{Domain, Protocol, Socket, Type};

use aquatic_udp_protocol::{IpVersion, Request, Response};
use udp_socket::{RecvMeta, Transmit, BATCH_SIZE};

use crate::common::network::ConnectionMap;
use crate::common::*;
use crate::config::Config;

use super::common::*;

pub fn run_socket_worker(
    state: State,
    config: Config,
    token_num: usize,
    request_sender: Sender<(ConnectedRequest, SocketAddr)>,
    response_receiver: Receiver<(ConnectedResponse, SocketAddr)>,
    num_bound_sockets: Arc<AtomicUsize>,
) {
    let mut rng = StdRng::from_entropy();

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
                    &request_sender,
                    &mut local_responses,
                );
            }
        }

        send_responses(
            &state,
            &config,
            &mut socket,
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

    let socket = socket.into();

    udp_socket::unix::init(&socket).expect("udp_socket init");

    socket
}

#[inline]
fn read_requests(
    config: &Config,
    state: &State,
    connections: &mut ConnectionMap,
    rng: &mut StdRng,
    socket: &mut UdpSocket,
    request_sender: &Sender<(ConnectedRequest, SocketAddr)>,
    local_responses: &mut Vec<(Response, SocketAddr)>,
) {
    let mut requests_received: usize = 0;
    let mut bytes_received: usize = 0;

    let valid_until = ValidUntil::new(config.cleaning.max_connection_age);
    let access_list_mode = config.access_list.mode;

    let mut storage = [[0u8; 1200]; BATCH_SIZE];

    let mut buffers = setup_buffers(&mut storage);
    let mut meta = [RecvMeta::default(); BATCH_SIZE];

    loop {
        // Informing mio is not necessary on Linux/macOS
        match udp_socket::unix::recv(socket, &mut buffers, &mut meta) {
            Ok(n) => {
                for (meta, buf) in meta.iter().zip(buffers.iter()).take(n) {
                    let request =
                        Request::from_bytes(&buf[..meta.len], config.protocol.max_scrape_torrents);

                    bytes_received += meta.len;

                    if request.is_ok() {
                        requests_received += 1;
                    }

                    handle_request(
                        &state.access_list,
                        connections,
                        rng,
                        request_sender,
                        local_responses,
                        access_list_mode,
                        valid_until,
                        request,
                        meta.source
                    );
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

fn handle_request(
    access_list: &Arc<AccessListArcSwap>,
    connections: &mut ConnectionMap,
    rng: &mut StdRng,
    request_sender: &Sender<(ConnectedRequest, SocketAddr)>,
    local_responses: &mut Vec<(Response, SocketAddr)>,
    access_list_mode: AccessListMode,
    valid_until: ValidUntil,
    request: Result<Request, RequestParseError>,
    src: SocketAddr,
) {
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
                if access_list.allows(access_list_mode, &request.info_hash.0) {
                    if let Err(err) = request_sender
                        .try_send((ConnectedRequest::Announce(request), src))
                    {
                        ::log::warn!("request_sender.try_send failed: {:?}", err)
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
                    request_sender.try_send((ConnectedRequest::Scrape(request), src))
                {
                    ::log::warn!("request_sender.try_send failed: {:?}", err)
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

#[inline]
fn send_responses(
    state: &State,
    config: &Config,
    socket: &mut UdpSocket,
    response_receiver: &Receiver<(ConnectedResponse, SocketAddr)>,
    local_responses: Drain<(Response, SocketAddr)>,
) {
    let mut responses_sent: usize = 0;

    let response_iterator = local_responses.into_iter().chain(
        response_receiver
            .try_iter()
            .map(|(response, addr)| (response.into(), addr)),
    );

    let mut transmits: Vec<Transmit> = Vec::new();
    
    for (response, addr) in response_iterator {
        let mut contents = Vec::new();

        let ip_version = ip_version_from_ip(addr.ip());

        match response.write(&mut contents, ip_version) {
            Ok(()) => {
                let transmit = Transmit {
                    destination: addr,
                    contents,
                    src_ip: None,
                    ecn: None,
                    segment_size: None,
                };

                transmits.push(transmit);
            }
            Err(err) => {
                ::log::error!("Response::write error: {:?}", err);
            }
        }
    }

    // Informing mio is not necessary on Linux/macOS
    match udp_socket::unix::send(socket, &transmits[..]) {
        Ok(num_sent) => {
            responses_sent += num_sent;
        }
        Err(err) => {
            ::log::warn!("socket send error: {:?}", err);
        }
    }

    if config.statistics.interval != 0 {
        state
            .statistics
            .responses_sent
            .fetch_add(responses_sent, Ordering::SeqCst);
    }
}

fn setup_buffers<'a>(storage: &'a mut [[u8; 1200]; BATCH_SIZE]) -> Vec<IoSliceMut<'a>> {
    let mut buffers = Vec::with_capacity(BATCH_SIZE);
    let mut rest = &mut storage[..];

    for _ in 0..BATCH_SIZE {
        let (b, r) = rest.split_at_mut(1);
        rest = r;
        buffers.push(IoSliceMut::new(&mut b[0]));
    }

    buffers
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
