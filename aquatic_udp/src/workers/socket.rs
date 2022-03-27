use std::collections::BTreeMap;
use std::io::{Cursor, ErrorKind};
use std::sync::{
    atomic::{AtomicUsize, Ordering},
    Arc,
};
use std::time::{Duration, Instant};
use std::vec::Drain;

use crossbeam_channel::{Receiver, Sender};
use hdrhistogram::Histogram;
use mio::net::UdpSocket;
use mio::{Events, Interest, Poll, Token};
use rand::prelude::{Rng, SeedableRng, StdRng};
use slab::Slab;

use aquatic_common::access_list::create_access_list_cache;
use aquatic_common::access_list::AccessListCache;
use aquatic_common::ValidUntil;
use aquatic_common::{AmortizedIndexMap, CanonicalSocketAddr};
use aquatic_udp_protocol::*;
use socket2::{Domain, Protocol, Socket, Type};

use crate::common::*;
use crate::config::Config;

#[derive(Default)]
pub struct ConnectionMap(AmortizedIndexMap<(ConnectionId, CanonicalSocketAddr), ValidUntil>);

impl ConnectionMap {
    pub fn insert(
        &mut self,
        connection_id: ConnectionId,
        socket_addr: CanonicalSocketAddr,
        valid_until: ValidUntil,
    ) {
        self.0.insert((connection_id, socket_addr), valid_until);
    }

    pub fn contains(&self, connection_id: ConnectionId, socket_addr: CanonicalSocketAddr) -> bool {
        self.0.contains_key(&(connection_id, socket_addr))
    }

    pub fn clean(&mut self) {
        let now = Instant::now();

        self.0.retain(|_, v| v.0 > now);
        self.0.shrink_to_fit();
    }
}

#[derive(Debug)]
pub struct PendingScrapeResponseSlabEntry {
    num_pending: usize,
    valid_until: ValidUntil,
    torrent_stats: BTreeMap<usize, TorrentScrapeStatistics>,
    transaction_id: TransactionId,
    tag: RequestTag,
}

#[derive(Default)]
pub struct PendingScrapeResponseSlab(Slab<PendingScrapeResponseSlabEntry>);

impl PendingScrapeResponseSlab {
    pub fn prepare_split_requests(
        &mut self,
        config: &Config,
        request: ScrapeRequest,
        tag: RequestTag,
        valid_until: ValidUntil,
    ) -> impl IntoIterator<Item = (RequestWorkerIndex, PendingScrapeRequest)> {
        let mut split_requests: AmortizedIndexMap<RequestWorkerIndex, PendingScrapeRequest> =
            Default::default();

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
            tag,
        });

        split_requests
    }

    pub fn add_and_get_finished(
        &mut self,
        response: PendingScrapeResponse,
    ) -> Option<(Response, RequestTag)> {
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

            Some((
                Response::Scrape(ScrapeResponse {
                    transaction_id: entry.transaction_id,
                    torrent_stats: entry.torrent_stats.into_values().collect(),
                }),
                entry.tag,
            ))
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
                    "Removing PendingScrapeResponseSlab entry while cleaning. {:?}: {:?}",
                    k,
                    v
                );
            }

            keep
        });
        self.0.shrink_to_fit();
    }
}

pub struct LatencyRecorder {
    instants: Slab<Instant>,
    // Tracks latency in microseconds
    histogram: Histogram<u64>,
}

impl LatencyRecorder {
    fn create_histogram() -> Histogram<u64> {
        Histogram::new_with_max(10u64.pow(12), 3).unwrap()
    }

    fn new() -> Self {
        Self {
            instants: Default::default(),
            histogram: Self::create_histogram(),
        }
    }

    fn request_received(&mut self) -> RequestTag {
        RequestTag(self.instants.insert(Instant::now()))
    }

    fn response_sent(&mut self, tag: RequestTag) {
        if let Some(insert_time) = self.instants.try_remove(tag.0) {
            let latency = (Instant::now() - insert_time).as_micros() as u64;

            self.histogram.saturating_record(latency);
        }
    }

    fn remove_tag(&mut self, tag: RequestTag) {
        self.instants.try_remove(tag.0);
    }

    fn extract_histogram(&mut self) -> Histogram<u64> {
        let mut histogram = Self::create_histogram();

        ::std::mem::swap(&mut self.histogram, &mut histogram);

        histogram
    }
}

pub fn run_socket_worker(
    state: State,
    config: Config,
    token_num: usize,
    request_sender: ConnectedRequestSender,
    response_receiver: Receiver<(ConnectedResponse, CanonicalSocketAddr)>,
    histogram_sender: Sender<Histogram<u64>>,
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
    let mut pending_scrape_responses = PendingScrapeResponseSlab::default();
    let mut access_list_cache = create_access_list_cache(&state.access_list);
    let mut latency_recorder = LatencyRecorder::new();

    let mut local_responses: Vec<(Response, RequestTag, CanonicalSocketAddr)> = Vec::new();

    let poll_timeout = Duration::from_millis(config.network.poll_timeout_ms);

    let connection_cleaning_duration =
        Duration::from_secs(config.cleaning.connection_cleaning_interval);
    let pending_scrape_cleaning_duration =
        Duration::from_secs(config.cleaning.pending_scrape_cleaning_interval);
    let statistics_update_interval = Duration::from_secs(config.statistics.interval);

    let mut connection_valid_until = ValidUntil::new(config.cleaning.max_connection_age);
    let mut pending_scrape_valid_until = ValidUntil::new(config.cleaning.max_pending_scrape_age);

    let mut last_connection_cleaning = Instant::now();
    let mut last_pending_scrape_cleaning = Instant::now();
    let mut last_histogram_sending = Instant::now();

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
                    &mut latency_recorder,
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
            &mut latency_recorder,
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
            if now > last_histogram_sending + statistics_update_interval {
                if let Err(err) = histogram_sender.try_send(latency_recorder.extract_histogram()) {
                    ::log::error!("Couldn't send latency data to statistics worker: {:#}", err);
                }

                last_histogram_sending = now;
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
    pending_scrape_responses: &mut PendingScrapeResponseSlab,
    access_list_cache: &mut AccessListCache,
    latency_recorder: &mut LatencyRecorder,
    rng: &mut StdRng,
    socket: &mut UdpSocket,
    buffer: &mut [u8],
    request_sender: &ConnectedRequestSender,
    local_responses: &mut Vec<(Response, RequestTag, CanonicalSocketAddr)>,
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
                let request_tag = latency_recorder.request_received();

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
                    connections,
                    pending_scrape_responses,
                    access_list_cache,
                    latency_recorder,
                    rng,
                    request_sender,
                    local_responses,
                    connection_valid_until,
                    pending_scrape_valid_until,
                    res_request,
                    src,
                    request_tag,
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
    pending_scrape_responses: &mut PendingScrapeResponseSlab,
    access_list_cache: &mut AccessListCache,
    latency_recorder: &mut LatencyRecorder,
    rng: &mut StdRng,
    request_sender: &ConnectedRequestSender,
    local_responses: &mut Vec<(Response, RequestTag, CanonicalSocketAddr)>,
    connection_valid_until: ValidUntil,
    pending_scrape_valid_until: ValidUntil,
    res_request: Result<Request, RequestParseError>,
    src: CanonicalSocketAddr,
    tag: RequestTag,
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

            local_responses.push((response, tag, src))
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
                        ConnectedRequest::Announce(request, tag),
                        src,
                    );
                } else {
                    let response = Response::Error(ErrorResponse {
                        transaction_id: request.transaction_id,
                        message: "Info hash not allowed".into(),
                    });

                    local_responses.push((response, tag, src))
                }
            }
        }
        Ok(Request::Scrape(request)) => {
            if connections.contains(request.connection_id, src) {
                let split_requests = pending_scrape_responses.prepare_split_requests(
                    config,
                    request,
                    tag,
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
                if connections.contains(connection_id, src) {
                    let response = ErrorResponse {
                        transaction_id,
                        message: err.right_or("Parse error").into(),
                    };

                    local_responses.push((response.into(), tag, src));
                }
            } else {
                latency_recorder.remove_tag(tag);
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
    response_receiver: &Receiver<(ConnectedResponse, CanonicalSocketAddr)>,
    pending_scrape_responses: &mut PendingScrapeResponseSlab,
    latency_recorder: &mut LatencyRecorder,
    local_responses: Drain<(Response, RequestTag, CanonicalSocketAddr)>,
) {
    for (response, tag, addr) in local_responses {
        send_response(
            state,
            config,
            socket,
            buffer,
            latency_recorder,
            response,
            tag,
            addr,
        );
    }

    for (response, addr) in response_receiver.try_iter() {
        let opt_response = match response {
            ConnectedResponse::Scrape(r) => pending_scrape_responses.add_and_get_finished(r),
            ConnectedResponse::AnnounceIpv4(r, tag) => Some((Response::AnnounceIpv4(r), tag)),
            ConnectedResponse::AnnounceIpv6(r, tag) => Some((Response::AnnounceIpv6(r), tag)),
        };

        if let Some((response, tag)) = opt_response {
            send_response(
                state,
                config,
                socket,
                buffer,
                latency_recorder,
                response,
                tag,
                addr,
            );
        }
    }
}

fn send_response(
    state: &State,
    config: &Config,
    socket: &mut UdpSocket,
    buffer: &mut [u8],
    latency_recorder: &mut LatencyRecorder,
    response: Response,
    tag: RequestTag,
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
                    latency_recorder.response_sent(tag);

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
                map.prepare_split_requests(&config, request.to_owned(), RequestTag(0), valid_until);

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
