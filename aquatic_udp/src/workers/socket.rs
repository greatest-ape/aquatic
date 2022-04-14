use std::collections::BTreeMap;
use std::io::{Cursor, ErrorKind};
use std::sync::atomic::Ordering;
use std::time::{Duration, Instant};
use std::vec::Drain;

use anyhow::Context;
use aquatic_common::privileges::PrivilegeDropper;
use crossbeam_channel::Receiver;
use hashbrown::HashMap;
use mio::net::UdpSocket;
use mio::{Events, Interest, Poll, Token};
use slab::Slab;

use aquatic_common::access_list::create_access_list_cache;
use aquatic_common::access_list::AccessListCache;
use aquatic_common::CanonicalSocketAddr;
use aquatic_common::{PanicSentinel, ValidUntil};
use aquatic_udp_protocol::*;
use socket2::{Domain, Protocol, Socket, Type};

use crate::common::*;
use crate::config::Config;

#[derive(Debug)]
pub struct PendingScrapeResponseSlabEntry {
    num_pending: usize,
    valid_until: ValidUntil,
    torrent_stats: BTreeMap<usize, TorrentScrapeStatistics>,
    transaction_id: TransactionId,
}

#[derive(Default)]
pub struct PendingScrapeResponseSlab(Slab<PendingScrapeResponseSlabEntry>);

impl PendingScrapeResponseSlab {
    pub fn prepare_split_requests(
        &mut self,
        config: &Config,
        request: ScrapeRequest,
        valid_until: ValidUntil,
    ) -> impl IntoIterator<Item = (RequestWorkerIndex, PendingScrapeRequest)> {
        let capacity = config.request_workers.min(request.info_hashes.len());
        let mut split_requests: HashMap<RequestWorkerIndex, PendingScrapeRequest> =
            HashMap::with_capacity(capacity);

        if request.info_hashes.is_empty() {
            ::log::warn!(
                "Attempted to prepare PendingScrapeResponseSlab entry with zero info hashes"
            );

            return split_requests;
        }

        let vacant_entry = self.0.vacant_entry();
        let slab_key = vacant_entry.key();

        for (i, info_hash) in request.info_hashes.into_iter().enumerate() {
            let split_request = split_requests
                .entry(RequestWorkerIndex::from_info_hash(&config, info_hash))
                .or_insert_with(|| PendingScrapeRequest {
                    slab_key,
                    info_hashes: BTreeMap::new(),
                });

            split_request.info_hashes.insert(i, info_hash);
        }

        vacant_entry.insert(PendingScrapeResponseSlabEntry {
            num_pending: split_requests.len(),
            valid_until,
            torrent_stats: Default::default(),
            transaction_id: request.transaction_id,
        });

        split_requests
    }

    pub fn add_and_get_finished(
        &mut self,
        response: PendingScrapeResponse,
    ) -> Option<ScrapeResponse> {
        let finished = if let Some(entry) = self.0.get_mut(response.slab_key) {
            entry.num_pending -= 1;

            entry
                .torrent_stats
                .extend(response.torrent_stats.into_iter());

            entry.num_pending == 0
        } else {
            ::log::warn!(
                "PendingScrapeResponseSlab.add didn't find entry for key {:?}",
                response.slab_key
            );

            false
        };

        if finished {
            let entry = self.0.remove(response.slab_key);

            Some(ScrapeResponse {
                transaction_id: entry.transaction_id,
                torrent_stats: entry.torrent_stats.into_values().collect(),
            })
        } else {
            None
        }
    }

    pub fn clean(&mut self) {
        let now = Instant::now();

        self.0.retain(|k, v| {
            if v.valid_until.0 > now {
                true
            } else {
                ::log::warn!(
                    "Unconsumed PendingScrapeResponseSlab entry. {:?}: {:?}",
                    k,
                    v
                );

                false
            }
        });

        self.0.shrink_to_fit();
    }
}

pub fn run_socket_worker(
    _sentinel: PanicSentinel,
    state: State,
    config: Config,
    token_num: usize,
    mut connection_validator: ConnectionValidator,
    request_sender: ConnectedRequestSender,
    response_receiver: Receiver<(ConnectedResponse, CanonicalSocketAddr)>,
    priv_dropper: PrivilegeDropper,
) {
    let mut buffer = [0u8; MAX_PACKET_SIZE];

    let mut socket =
        UdpSocket::from_std(create_socket(&config, priv_dropper).expect("create socket"));
    let mut poll = Poll::new().expect("create poll");

    let interests = Interest::READABLE;

    poll.registry()
        .register(&mut socket, Token(token_num), interests)
        .unwrap();

    let mut events = Events::with_capacity(config.network.poll_event_capacity);
    let mut pending_scrape_responses = PendingScrapeResponseSlab::default();
    let mut access_list_cache = create_access_list_cache(&state.access_list);

    let mut local_responses: Vec<(Response, CanonicalSocketAddr)> = Vec::new();

    let poll_timeout = Duration::from_millis(config.network.poll_timeout_ms);

    let pending_scrape_cleaning_duration =
        Duration::from_secs(config.cleaning.pending_scrape_cleaning_interval);

    let mut pending_scrape_valid_until = ValidUntil::new(config.cleaning.max_pending_scrape_age);
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
                    &mut connection_validator,
                    &mut pending_scrape_responses,
                    &mut access_list_cache,
                    &mut socket,
                    &mut buffer,
                    &request_sender,
                    &mut local_responses,
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
        if iter_counter % 256 == 0 {
            let now = Instant::now();

            pending_scrape_valid_until =
                ValidUntil::new_with_now(now, config.cleaning.max_pending_scrape_age);

            if now > last_pending_scrape_cleaning + pending_scrape_cleaning_duration {
                pending_scrape_responses.clean();

                last_pending_scrape_cleaning = now;
            }
        }

        iter_counter = iter_counter.wrapping_add(1);
    }
}

fn read_requests(
    config: &Config,
    state: &State,
    connection_validator: &mut ConnectionValidator,
    pending_scrape_responses: &mut PendingScrapeResponseSlab,
    access_list_cache: &mut AccessListCache,
    socket: &mut UdpSocket,
    buffer: &mut [u8],
    request_sender: &ConnectedRequestSender,
    local_responses: &mut Vec<(Response, CanonicalSocketAddr)>,
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

                let src = CanonicalSocketAddr::new(src);

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
                    connection_validator,
                    pending_scrape_responses,
                    access_list_cache,
                    request_sender,
                    local_responses,
                    pending_scrape_valid_until,
                    res_request,
                    src,
                );
            }
            Err(err) if err.kind() == ErrorKind::WouldBlock => {
                break;
            }
            Err(err) => {
                ::log::warn!("recv_from error: {:#}", err);
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
    connection_validator: &mut ConnectionValidator,
    pending_scrape_responses: &mut PendingScrapeResponseSlab,
    access_list_cache: &mut AccessListCache,
    request_sender: &ConnectedRequestSender,
    local_responses: &mut Vec<(Response, CanonicalSocketAddr)>,
    pending_scrape_valid_until: ValidUntil,
    res_request: Result<Request, RequestParseError>,
    src: CanonicalSocketAddr,
) {
    let access_list_mode = config.access_list.mode;

    match res_request {
        Ok(Request::Connect(request)) => {
            let connection_id = connection_validator.create_connection_id(src);

            let response = Response::Connect(ConnectResponse {
                connection_id,
                transaction_id: request.transaction_id,
            });

            local_responses.push((response, src))
        }
        Ok(Request::Announce(request)) => {
            if connection_validator.connection_id_valid(src, request.connection_id) {
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
            if connection_validator.connection_id_valid(src, request.connection_id) {
                let split_requests = pending_scrape_responses.prepare_split_requests(
                    config,
                    request,
                    pending_scrape_valid_until,
                );

                for (request_worker_index, request) in split_requests {
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
                if connection_validator.connection_id_valid(src, connection_id) {
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

fn send_responses(
    state: &State,
    config: &Config,
    socket: &mut UdpSocket,
    buffer: &mut [u8],
    response_receiver: &Receiver<(ConnectedResponse, CanonicalSocketAddr)>,
    pending_scrape_responses: &mut PendingScrapeResponseSlab,
    local_responses: Drain<(Response, CanonicalSocketAddr)>,
) {
    for (response, addr) in local_responses {
        send_response(state, config, socket, buffer, response, addr);
    }

    for (response, addr) in response_receiver.try_iter() {
        match response {
            ConnectedResponse::Scrape(r) => {
                if let Some(response) = pending_scrape_responses.add_and_get_finished(r) {
                    send_response(
                        state,
                        config,
                        socket,
                        buffer,
                        Response::Scrape(response),
                        addr,
                    );
                }
            }
            ConnectedResponse::AnnounceIpv4(r) => {
                send_response(
                    state,
                    config,
                    socket,
                    buffer,
                    Response::AnnounceIpv4(r),
                    addr,
                );
            }
            ConnectedResponse::AnnounceIpv6(r) => {
                send_response(
                    state,
                    config,
                    socket,
                    buffer,
                    Response::AnnounceIpv6(r),
                    addr,
                );
            }
        };
    }
}

fn send_response(
    state: &State,
    config: &Config,
    socket: &mut UdpSocket,
    buffer: &mut [u8],
    response: Response,
    addr: CanonicalSocketAddr,
) {
    let mut cursor = Cursor::new(buffer);

    let canonical_addr_is_ipv4 = addr.is_ipv4();

    let addr = if config.network.address.is_ipv4() {
        addr.get_ipv4()
            .expect("found peer ipv6 address while running bound to ipv4 address")
    } else {
        addr.get_ipv6_mapped()
    };

    match response.write(&mut cursor) {
        Ok(()) => {
            let amt = cursor.position() as usize;

            match socket.send_to(&cursor.get_ref()[..amt], addr) {
                Ok(amt) if config.statistics.active() => {
                    let stats = if canonical_addr_is_ipv4 {
                        &state.statistics_ipv4
                    } else {
                        &state.statistics_ipv6
                    };

                    stats.bytes_sent.fetch_add(amt, Ordering::Relaxed);

                    match response {
                        Response::Connect(_) => {
                            stats.responses_sent_connect.fetch_add(1, Ordering::Relaxed);
                        }
                        Response::AnnounceIpv4(_) | Response::AnnounceIpv6(_) => {
                            stats
                                .responses_sent_announce
                                .fetch_add(1, Ordering::Relaxed);
                        }
                        Response::Scrape(_) => {
                            stats.responses_sent_scrape.fetch_add(1, Ordering::Relaxed);
                        }
                        Response::Error(_) => {
                            stats.responses_sent_error.fetch_add(1, Ordering::Relaxed);
                        }
                    }
                }
                Ok(_) => {}
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

pub fn create_socket(
    config: &Config,
    priv_dropper: PrivilegeDropper,
) -> anyhow::Result<::std::net::UdpSocket> {
    let socket = if config.network.address.is_ipv4() {
        Socket::new(Domain::IPV4, Type::DGRAM, Some(Protocol::UDP))?
    } else {
        Socket::new(Domain::IPV6, Type::DGRAM, Some(Protocol::UDP))?
    };

    if config.network.only_ipv6 {
        socket
            .set_only_v6(true)
            .with_context(|| "socket: set only ipv6")?;
    }

    socket
        .set_reuse_port(true)
        .with_context(|| "socket: set reuse port")?;

    socket
        .set_nonblocking(true)
        .with_context(|| "socket: set nonblocking")?;

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

    socket
        .bind(&config.network.address.into())
        .with_context(|| format!("socket: bind to {}", config.network.address))?;

    priv_dropper.after_socket_creation()?;

    Ok(socket.into())
}

#[cfg(test)]
mod tests {
    use quickcheck::TestResult;
    use quickcheck_macros::quickcheck;

    use super::*;

    #[quickcheck]
    fn test_pending_scrape_response_map(
        request_data: Vec<(i32, i64, u8)>,
        request_workers: u8,
    ) -> TestResult {
        if request_workers == 0 {
            return TestResult::discard();
        }

        let mut config = Config::default();

        config.request_workers = request_workers as usize;

        let valid_until = ValidUntil::new(1);

        let mut map = PendingScrapeResponseSlab::default();

        let mut requests = Vec::new();

        for (t, c, b) in request_data {
            if b == 0 {
                return TestResult::discard();
            }

            let mut info_hashes = Vec::new();

            for i in 0..b {
                let info_hash = InfoHash([i; 20]);

                info_hashes.push(info_hash);
            }

            let request = ScrapeRequest {
                transaction_id: TransactionId(t),
                connection_id: ConnectionId(c),
                info_hashes,
            };

            requests.push(request);
        }

        let mut all_split_requests = Vec::new();

        for request in requests.iter() {
            let split_requests =
                map.prepare_split_requests(&config, request.to_owned(), valid_until);

            all_split_requests.push(
                split_requests
                    .into_iter()
                    .collect::<Vec<(RequestWorkerIndex, PendingScrapeRequest)>>(),
            );
        }

        assert_eq!(map.0.len(), requests.len());

        let mut responses = Vec::new();

        for split_requests in all_split_requests {
            for (worker_index, split_request) in split_requests {
                assert!(worker_index.0 < request_workers as usize);

                let torrent_stats = split_request
                    .info_hashes
                    .into_iter()
                    .map(|(i, info_hash)| {
                        (
                            i,
                            TorrentScrapeStatistics {
                                seeders: NumberOfPeers((info_hash.0[0]) as i32),
                                leechers: NumberOfPeers(0),
                                completed: NumberOfDownloads(0),
                            },
                        )
                    })
                    .collect();

                let response = PendingScrapeResponse {
                    slab_key: split_request.slab_key,
                    torrent_stats,
                };

                if let Some(response) = map.add_and_get_finished(response) {
                    responses.push(response);
                }
            }
        }

        assert!(map.0.is_empty());
        assert_eq!(responses.len(), requests.len());

        TestResult::from_bool(true)
    }
}
