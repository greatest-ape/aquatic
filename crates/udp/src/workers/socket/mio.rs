use std::io::{Cursor, ErrorKind};
use std::marker::PhantomData;
use std::sync::atomic::Ordering;
use std::time::Duration;

use anyhow::Context;
use aquatic_common::access_list::AccessListCache;
use crossbeam_channel::Sender;
use mio::net::UdpSocket;
use mio::{Events, Interest, Poll, Token};

use aquatic_common::{
    access_list::create_access_list_cache, privileges::PrivilegeDropper, CanonicalSocketAddr,
    ValidUntil,
};
use aquatic_udp_protocol::*;
use rand::rngs::SmallRng;
use rand::SeedableRng;

use crate::common::*;
use crate::config::Config;

use super::validator::ConnectionValidator;
use super::{create_socket, EXTRA_PACKET_SIZE_IPV4, EXTRA_PACKET_SIZE_IPV6};

struct SocketWorker;

impl SocketWorker {
    pub fn run(
        config: Config,
        shared_state: State,
        statistics: CachePaddedArc<IpVersionStatistics<SocketWorkerStatistics>>,
        statistics_sender: Sender<StatisticsMessage>,
        validator: ConnectionValidator,
        priv_dropper: PrivilegeDropper,
    ) -> anyhow::Result<()> {
        let socket = UdpSocket::from_std(create_socket(&config, priv_dropper)?);
        let access_list_cache = create_access_list_cache(&shared_state.access_list);
        let peer_valid_until = ValidUntil::new(
            shared_state.server_start_instant,
            config.cleaning.max_peer_age,
        );

        let mut shared = WorkerSharedData {
            config,
            shared_state,
            statistics,
            statistics_sender,
            validator,
            access_list_cache,
            buffer: [0; BUFFER_SIZE],
            rng: SmallRng::from_entropy(),
            peer_valid_until,
        };

        let mut opt_resend_buffer =
            (shared.config.network.resend_buffer_max_len > 0).then_some(Vec::new());
        let mut events = Events::with_capacity(1);
        let mut poll = Poll::new().context("create poll")?;

        poll.registry()
            .register(&mut self.socket, Token(0), Interest::READABLE)
            .context("register poll")?;

        let poll_timeout = Duration::from_millis(self.config.network.poll_timeout_ms);

        let mut iter_counter = 0u64;

        loop {
            poll.poll(&mut events, Some(poll_timeout)).context("poll")?;

            for event in events.iter() {
                if event.is_readable() {
                    self.read_and_handle_requests(&mut opt_resend_buffer);
                }
            }

            // FIXME: resend buffer

            if iter_counter % 256 == 0 {
                shared.validator.update_elapsed();

                shared.peer_valid_until = ValidUntil::new(
                    shared.shared_state.server_start_instant,
                    shared.config.cleaning.max_peer_age,
                );
            }

            iter_counter = iter_counter.wrapping_add(1);
        }
    }
}

struct WorkerSharedData {
    config: Config,
    shared_state: State,
    statistics: CachePaddedArc<IpVersionStatistics<SocketWorkerStatistics>>,
    statistics_sender: Sender<StatisticsMessage>,
    access_list_cache: AccessListCache,
    validator: ConnectionValidator,
    buffer: [u8; BUFFER_SIZE],
    rng: SmallRng,
    peer_valid_until: ValidUntil,
}

impl WorkerSharedData {
    fn handle_request(&mut self, request: Request, src: CanonicalSocketAddr) -> Option<Response> {
        let access_list_mode = self.config.access_list.mode;

        match request {
            Request::Connect(request) => {
                return Some(Response::Connect(ConnectResponse {
                    connection_id: self.validator.create_connection_id(src),
                    transaction_id: request.transaction_id,
                }));
            }
            Request::Announce(request) => {
                if self
                    .validator
                    .connection_id_valid(src, request.connection_id)
                {
                    if self
                        .access_list_cache
                        .load()
                        .allows(access_list_mode, &request.info_hash.0)
                    {
                        let response = self.shared_state.torrent_maps.announce(
                            &self.config,
                            &self.statistics_sender,
                            &mut self.rng,
                            &request,
                            src,
                            self.peer_valid_until,
                        );

                        return Some(response);
                    } else {
                        return Some(Response::Error(ErrorResponse {
                            transaction_id: request.transaction_id,
                            message: "Info hash not allowed".into(),
                        }));
                    }
                }
            }
            Request::Scrape(request) => {
                if self
                    .validator
                    .connection_id_valid(src, request.connection_id)
                {
                    return Some(Response::Scrape(
                        self.shared_state.torrent_maps.scrape(request, src),
                    ));
                }
            }
        }

        None
    }
}

struct IPV4;
struct IPV6;

struct Socket<T> {
    socket: UdpSocket,
    opt_resend_buffer: Option<Vec<(CanonicalSocketAddr, Response)>>,
    phantom_data: PhantomData<T>,
}

impl Socket<IPV4> {
    fn read_and_handle_requests(&mut self
    , shared: &mut WorkerSharedData
    ) {
        let max_scrape_torrents = shared.config.protocol.max_scrape_torrents;

        loop {
            match self.socket.recv_from(&mut shared.buffer[..]) {
                Ok((bytes_read, src)) => {
                    let src_port = src.port();
                    let src = CanonicalSocketAddr::new(src);

                    // Use canonical address for statistics
                    let opt_statistics = if shared.config.statistics.active() {
                        if src.is_ipv4() {
                            let statistics = &shared.statistics.ipv4;

                            statistics
                                .bytes_received
                                .fetch_add(bytes_read + EXTRA_PACKET_SIZE_IPV4, Ordering::Relaxed);

                            Some(statistics)
                        } else {
                            let statistics = &shared.statistics.ipv6;

                            statistics
                                .bytes_received
                                .fetch_add(bytes_read + EXTRA_PACKET_SIZE_IPV6, Ordering::Relaxed);

                            Some(statistics)
                        }
                    } else {
                        None
                    };

                    if src_port == 0 {
                        ::log::debug!("Ignored request because source port is zero");

                        continue;
                    }

                    match Request::parse_bytes(&shared.buffer[..bytes_read], max_scrape_torrents) {
                        Ok(request) => {
                            if let Some(statistics) = opt_statistics {
                                statistics.requests.fetch_add(1, Ordering::Relaxed);
                            }

                            if let Some(response) = shared.handle_request(request, src) {
                                self.send_response (shared, src, response, false);
                            }
                        }
                        Err(RequestParseError::Sendable {
                            connection_id,
                            transaction_id,
                            err,
                        }) if shared.validator.connection_id_valid(src, connection_id) => {
                            let response = ErrorResponse {
                                transaction_id,
                                message: err.into(),
                            };

                            self.send_response( shared, src, Response::Error(response), false);

                            ::log::debug!("request parse error (sent error response): {:?}", err);
                        }
                        Err(err) => {
                            ::log::debug!(
                                "request parse error (didn't send error response): {:?}",
                                err
                            );
                        }
                    };
                }
                Err(err) if err.kind() == ErrorKind::WouldBlock => {
                    break;
                }
                Err(err) => {
                    ::log::warn!("recv_from error: {:#}", err);
                }
            }
        }
    }
    fn send_response(
        &mut self,
        shared: &mut WorkerSharedData,
        canonical_addr: CanonicalSocketAddr,
        response: Response,
        disable_resend_buffer: bool,
    ) {
        let mut buffer = Cursor::new(&mut shared.buffer[..]);

        if let Err(err) = response.write_bytes(&mut buffer) {
            ::log::error!("failed writing response to buffer: {:#}", err);

            return;
        }

        let bytes_written = buffer.position() as usize;

        let addr = if shared.config.network.address.is_ipv4() {
            canonical_addr
                .get_ipv4()
                .expect("found peer ipv6 address while running bound to ipv4 address")
        } else {
            canonical_addr.get_ipv6_mapped()
        };

        match self
            .socket
            .send_to(&buffer.into_inner()[..bytes_written], addr)
        {
            Ok(bytes_sent) if shared.config.statistics.active() => {
                let stats = if canonical_addr.is_ipv4() {
                    let stats = &shared.statistics.ipv4;

                    stats
                        .bytes_sent
                        .fetch_add(bytes_sent + EXTRA_PACKET_SIZE_IPV4, Ordering::Relaxed);

                    stats
                } else {
                    let stats = &shared.statistics.ipv6;

                    stats
                        .bytes_sent
                        .fetch_add(bytes_sent + EXTRA_PACKET_SIZE_IPV6, Ordering::Relaxed);

                    stats
                };

                match response {
                    Response::Connect(_) => {
                        stats.responses_connect.fetch_add(1, Ordering::Relaxed);
                    }
                    Response::AnnounceIpv4(_) | Response::AnnounceIpv6(_) => {
                        stats.responses_announce.fetch_add(1, Ordering::Relaxed);
                    }
                    Response::Scrape(_) => {
                        stats.responses_scrape.fetch_add(1, Ordering::Relaxed);
                    }
                    Response::Error(_) => {
                        stats.responses_error.fetch_add(1, Ordering::Relaxed);
                    }
                }
            }
            Ok(_) => (),
            Err(err) => match self.opt_resend_buffer.as_mut() {
                Some(resend_buffer)
                    if !disable_resend_buffer && (err.raw_os_error() == Some(libc::ENOBUFS))
                        || (err.kind() == ErrorKind::WouldBlock) =>
                {
                    if resend_buffer.len() < shared.config.network.resend_buffer_max_len {
                        ::log::debug!("Adding response to resend queue, since sending it to {} failed with: {:#}", addr, err);

                        resend_buffer.push((canonical_addr, response));
                    } else {
                        ::log::warn!("Response resend buffer full, dropping response");
                    }
                }
                _ => {
                    ::log::warn!("Sending response to {} failed: {:#}", addr, err);
                }
            },
        }

        ::log::debug!("send response fn finished");
    }


    /// If resend buffer is enabled, send any responses in it
    fn resend_failed(&mut self, shared: &mut WorkerSharedData) {
        if self.opt_resend_buffer.is_some() {
            let mut tmp_resend_buffer = Vec::new();

            // Do memory swap shenanigans to get around false positive in
            // borrow checker regarding double mut borrowing of self

            if let Some(resend_buffer) = self.opt_resend_buffer.as_mut() {
                ::std::mem::swap(resend_buffer, &mut tmp_resend_buffer);
            }

            for (addr, response) in tmp_resend_buffer.drain(..) {
                self.send_response(shared, addr, response, true);
            }

            if let Some(resend_buffer) = self.opt_resend_buffer.as_mut() {
                ::std::mem::swap(resend_buffer, &mut tmp_resend_buffer);
            }
        }
    }
}