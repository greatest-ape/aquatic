use std::collections::BTreeMap;
use std::io::{Cursor, ErrorKind};
use std::net::{Ipv4Addr, SocketAddr, SocketAddrV4, SocketAddrV6};
use std::sync::{
    atomic::{AtomicUsize, Ordering},
    Arc,
};
use std::time::{Duration, Instant};
use std::vec::Drain;

use crossbeam_channel::Receiver;
use mio::net::UdpSocket;
use mio::{Events, Interest, Poll, Token};
use rand::prelude::{Rng, SeedableRng, StdRng};

use aquatic_common::access_list::create_access_list_cache;
use aquatic_common::access_list::AccessListCache;
use aquatic_common::AHashIndexMap;
use aquatic_common::ValidUntil;
use aquatic_udp_protocol::*;
use socket2::{Domain, Protocol, Socket, Type};

use crate::common::*;
use crate::config::Config;

#[derive(Default)]
pub struct ConnectionMap(AHashIndexMap<(ConnectionId, SocketAddr), ValidUntil>);

impl ConnectionMap {
    pub fn insert(
        &mut self,
        connection_id: ConnectionId,
        socket_addr: SocketAddr,
        valid_until: ValidUntil,
    ) {
        self.0.insert((connection_id, socket_addr), valid_until);
    }

    pub fn contains(&self, connection_id: ConnectionId, socket_addr: SocketAddr) -> bool {
        self.0.contains_key(&(connection_id, socket_addr))
    }

    pub fn clean(&mut self) {
        let now = Instant::now();

        self.0.retain(|_, v| v.0 > now);
        self.0.shrink_to_fit();
    }
}

#[derive(Debug)]
pub struct PendingScrapeResponseMapEntry {
    num_pending: usize,
    valid_until: ValidUntil,
    torrent_stats: BTreeMap<usize, TorrentScrapeStatistics>,
}

#[derive(Default)]
pub struct PendingScrapeResponseMap(
    AHashIndexMap<(ConnectionId, TransactionId), PendingScrapeResponseMapEntry>,
);

impl PendingScrapeResponseMap {
    pub fn prepare(
        &mut self,
        connection_id: ConnectionId,
        transaction_id: TransactionId,
        num_pending: usize,
        valid_until: ValidUntil,
    ) {
        if num_pending == 0 {
            ::log::warn!("Attempted to prepare PendingScrapeResponseMap entry with num_pending=0");

            return;
        }

        let entry = PendingScrapeResponseMapEntry {
            num_pending,
            valid_until,
            torrent_stats: Default::default(),
        };

        self.0.insert((connection_id, transaction_id), entry);
    }

    pub fn add_and_get_finished(&mut self, response: PendingScrapeResponse) -> Option<Response> {
        let key = (response.connection_id, response.transaction_id);

        let finished = if let Some(entry) = self.0.get_mut(&key) {
            entry.num_pending -= 1;

            entry.torrent_stats.extend(response.torrent_stats.into_iter());

            entry.num_pending == 0
        } else {
            ::log::warn!("PendingScrapeResponseMap.add didn't find entry for key {:?}", key);

            false
        };

        if finished {
            let entry = self.0.remove(&key).unwrap();

            Some(Response::Scrape(ScrapeResponse {
                transaction_id: response.transaction_id,
                torrent_stats: entry.torrent_stats.into_values().collect(),
            }))
        } else {
            None
        }
    }

    pub fn clean(&mut self) {
        let now = Instant::now();

        self.0.retain(|k, v| {
            let keep = v.valid_until.0 > now;
            
            if !keep {
                ::log::warn!(
                    "Removing PendingScrapeResponseMap entry while cleaning. {:?}: {:?}",
                    k,
                    v
                );
            }

            keep
        });
        self.0.shrink_to_fit();
    }
}

pub fn run_socket_worker(
    state: State,
    config: Config,
    token_num: usize,
    request_sender: ConnectedRequestSender,
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
    let mut pending_scrape_responses = PendingScrapeResponseMap::default();
    let mut access_list_cache = create_access_list_cache(&state.access_list);

    let mut local_responses: Vec<(Response, SocketAddr)> = Vec::new();

    let poll_timeout = Duration::from_millis(config.network.poll_timeout_ms);

    let connection_cleaning_duration =
        Duration::from_secs(config.cleaning.connection_cleaning_interval);
    let pending_scrape_cleaning_duration =
        Duration::from_secs(config.cleaning.pending_scrape_cleaning_interval);

    let mut connection_valid_until = ValidUntil::new(config.cleaning.max_connection_age);
    let mut pending_scrape_valid_until = ValidUntil::new(config.cleaning.max_pending_scrape_age);

    let mut last_connection_cleaning = Instant::now();
    let mut last_pending_scrape_cleaning = Instant::now();

    let mut iter_counter = 0usize;

    loop {
        poll.poll(&mut events, Some(poll_timeout))
            .expect("failed polling");

        for event in events.iter() {
            let token = event.token();

            if (token.0 == token_num) & event.is_readable() {
                read_requests(
                    &config,
                    &state,
                    &mut connections,
                    &mut pending_scrape_responses,
                    &mut access_list_cache,
                    &mut rng,
                    &mut socket,
                    &mut buffer,
                    &request_sender,
                    &mut local_responses,
                    connection_valid_until,
                    pending_scrape_valid_until,
                );
            }
        }

        send_responses(
            &state,
            &config,
            &mut socket,
            &mut buffer,
            &response_receiver,
            &mut pending_scrape_responses,
            local_responses.drain(..),
        );

        // Run periodic ValidUntil updates and state cleaning
        if iter_counter % 128 == 0 {
            let now = Instant::now();

            connection_valid_until =
                ValidUntil::new_with_now(now, config.cleaning.max_connection_age);
            pending_scrape_valid_until =
                ValidUntil::new_with_now(now, config.cleaning.max_pending_scrape_age);

            if now > last_connection_cleaning + connection_cleaning_duration {
                connections.clean();

                last_connection_cleaning = now;
            }
            if now > last_pending_scrape_cleaning + pending_scrape_cleaning_duration {
                pending_scrape_responses.clean();

                last_pending_scrape_cleaning = now;
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
    pending_scrape_responses: &mut PendingScrapeResponseMap,
    access_list_cache: &mut AccessListCache,
    rng: &mut StdRng,
    socket: &mut UdpSocket,
    buffer: &mut [u8],
    request_sender: &ConnectedRequestSender,
    local_responses: &mut Vec<(Response, SocketAddr)>,
    connection_valid_until: ValidUntil,
    pending_scrape_valid_until: ValidUntil,
) {
    let mut requests_received_ipv4: usize = 0;
    let mut requests_received_ipv6: usize = 0;
    let mut bytes_received_ipv4: usize = 0;
    let mut bytes_received_ipv6 = 0;

    loop {
        match socket.recv_from(&mut buffer[..]) {
            Ok((amt, src)) => {
                let res_request =
                    Request::from_bytes(&buffer[..amt], config.protocol.max_scrape_torrents);

                let src = match src {
                    src @ SocketAddr::V4(_) => src,
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
                };

                // Update statistics for converted address
                if src.is_ipv4() {
                    if res_request.is_ok() {
                        requests_received_ipv4 += 1;
                    }
                    bytes_received_ipv4 += amt;
                } else {
                    if res_request.is_ok() {
                        requests_received_ipv6 += 1;
                    }
                    bytes_received_ipv6 += amt;
                }

                handle_request(
                    config,
                    connections,
                    pending_scrape_responses,
                    access_list_cache,
                    rng,
                    request_sender,
                    local_responses,
                    connection_valid_until,
                    pending_scrape_valid_until,
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

    if config.statistics.active() {
        state
            .statistics_ipv4
            .requests_received
            .fetch_add(requests_received_ipv4, Ordering::Release);
        state
            .statistics_ipv6
            .requests_received
            .fetch_add(requests_received_ipv6, Ordering::Release);
        state
            .statistics_ipv4
            .bytes_received
            .fetch_add(bytes_received_ipv4, Ordering::Release);
        state
            .statistics_ipv6
            .bytes_received
            .fetch_add(bytes_received_ipv6, Ordering::Release);
    }
}

pub fn handle_request(
    config: &Config,
    connections: &mut ConnectionMap,
    pending_scrape_responses: &mut PendingScrapeResponseMap,
    access_list_cache: &mut AccessListCache,
    rng: &mut StdRng,
    request_sender: &ConnectedRequestSender,
    local_responses: &mut Vec<(Response, SocketAddr)>,
    connection_valid_until: ValidUntil,
    pending_scrape_valid_until: ValidUntil,
    res_request: Result<Request, RequestParseError>,
    src: SocketAddr,
) {
    let access_list_mode = config.access_list.mode;

    match res_request {
        Ok(Request::Connect(request)) => {
            let connection_id = ConnectionId(rng.gen());

            connections.insert(connection_id, src, connection_valid_until);

            let response = Response::Connect(ConnectResponse {
                connection_id,
                transaction_id: request.transaction_id,
            });

            local_responses.push((response, src))
        }
        Ok(Request::Announce(request)) => {
            if connections.contains(request.connection_id, src) {
                if access_list_cache
                    .load()
                    .allows(access_list_mode, &request.info_hash.0)
                {
                    let worker_index =
                        RequestWorkerIndex::from_info_hash(config, request.info_hash);

                    request_sender.try_send_to(
                        worker_index,
                        ConnectedRequest::Announce(request),
                        src,
                    );
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
                let mut requests: AHashIndexMap<RequestWorkerIndex, PendingScrapeRequest> =
                    Default::default();

                let connection_id = request.connection_id;
                let transaction_id = request.transaction_id;

                for (i, info_hash) in request.info_hashes.into_iter().enumerate() {
                    let pending = requests
                        .entry(RequestWorkerIndex::from_info_hash(&config, info_hash))
                        .or_insert_with(|| PendingScrapeRequest {
                            connection_id,
                            transaction_id,
                            info_hashes: BTreeMap::new(),
                        });

                    pending.info_hashes.insert(i, info_hash);
                }

                pending_scrape_responses.prepare(
                    connection_id,
                    transaction_id,
                    requests.len(),
                    pending_scrape_valid_until,
                );

                for (request_worker_index, request) in requests {
                    request_sender.try_send_to(
                        request_worker_index,
                        ConnectedRequest::Scrape(request),
                        src,
                    );
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
    buffer: &mut [u8],
    response_receiver: &Receiver<(ConnectedResponse, SocketAddr)>,
    pending_scrape_responses: &mut PendingScrapeResponseMap,
    local_responses: Drain<(Response, SocketAddr)>,
) {
    let mut responses_sent_ipv4: usize = 0;
    let mut responses_sent_ipv6: usize = 0;
    let mut bytes_sent_ipv4: usize = 0;
    let mut bytes_sent_ipv6: usize = 0;

    for (response, addr) in local_responses {
        send_response(
            config,
            socket,
            buffer,
            &mut responses_sent_ipv4,
            &mut responses_sent_ipv6,
            &mut bytes_sent_ipv4,
            &mut bytes_sent_ipv6,
            response,
            addr,
        );
    }

    for (response, addr) in response_receiver.try_iter() {
        let opt_response = match response {
            ConnectedResponse::Scrape(r) => pending_scrape_responses.add_and_get_finished(r),
            ConnectedResponse::AnnounceIpv4(r) => Some(Response::AnnounceIpv4(r)),
            ConnectedResponse::AnnounceIpv6(r) => Some(Response::AnnounceIpv6(r)),
        };

        if let Some(response) = opt_response {
            send_response(
                config,
                socket,
                buffer,
                &mut responses_sent_ipv4,
                &mut responses_sent_ipv6,
                &mut bytes_sent_ipv4,
                &mut bytes_sent_ipv6,
                response,
                addr,
            );
        }
    }

    if config.statistics.active() {
        state
            .statistics_ipv4
            .responses_sent
            .fetch_add(responses_sent_ipv4, Ordering::Release);
        state
            .statistics_ipv6
            .responses_sent
            .fetch_add(responses_sent_ipv6, Ordering::Release);
        state
            .statistics_ipv4
            .bytes_sent
            .fetch_add(bytes_sent_ipv4, Ordering::Release);
        state
            .statistics_ipv6
            .bytes_sent
            .fetch_add(bytes_sent_ipv6, Ordering::Release);
    }
}

fn send_response(
    config: &Config,
    socket: &mut UdpSocket,
    buffer: &mut [u8],
    responses_sent_ipv4: &mut usize,
    responses_sent_ipv6: &mut usize,
    bytes_sent_ipv4: &mut usize,
    bytes_sent_ipv6: &mut usize,
    response: Response,
    addr: SocketAddr,
) {
    let mut cursor = Cursor::new(buffer);

    let addr_is_ipv4 = addr.is_ipv4();

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
                    if addr_is_ipv4 {
                        *responses_sent_ipv4 += 1;
                        *bytes_sent_ipv4 += amt;
                    } else {
                        *responses_sent_ipv6 += 1;
                        *bytes_sent_ipv6 += amt;
                    }
                }
                Err(err) => {
                    ::log::info!("send_to error: {}", err);
                }
            }
        }
        Err(err) => {
            ::log::error!("Response::write error: {:?}", err);
        }
    }
}

pub fn create_socket(config: &Config) -> ::std::net::UdpSocket {
    let socket = if config.network.address.is_ipv4() {
        Socket::new(Domain::IPV4, Type::DGRAM, Some(Protocol::UDP))
    } else {
        Socket::new(Domain::IPV6, Type::DGRAM, Some(Protocol::UDP))
    }
    .expect("create socket");

    if config.network.only_ipv6 {
        socket.set_only_v6(true).expect("socket: set only ipv6");
    }

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
