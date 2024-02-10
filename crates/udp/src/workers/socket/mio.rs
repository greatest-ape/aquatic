use std::io::{Cursor, ErrorKind};
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

pub struct SocketWorker {
    config: Config,
    shared_state: State,
    statistics: CachePaddedArc<IpVersionStatistics<SocketWorkerStatistics>>,
    statistics_sender: Sender<StatisticsMessage>,
    access_list_cache: AccessListCache,
    validator: ConnectionValidator,
    socket: UdpSocket,
    buffer: [u8; BUFFER_SIZE],
    rng: SmallRng,
}

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

        let mut worker = Self {
            config,
            shared_state,
            statistics,
            statistics_sender,
            validator,
            access_list_cache,
            socket,
            buffer: [0; BUFFER_SIZE],
            rng: SmallRng::from_entropy(),
        };

        worker.run_inner()
    }

    pub fn run_inner(&mut self) -> anyhow::Result<()> {
        let mut opt_resend_buffer =
            (self.config.network.resend_buffer_max_len > 0).then_some(Vec::new());
        let mut events = Events::with_capacity(1);
        let mut poll = Poll::new().context("create poll")?;

        poll.registry()
            .register(&mut self.socket, Token(0), Interest::READABLE)
            .context("register poll")?;

        let poll_timeout = Duration::from_millis(self.config.network.poll_timeout_ms);

        loop {
            poll.poll(&mut events, Some(poll_timeout)).context("poll")?;

            for event in events.iter() {
                if event.is_readable() {
                    self.read_and_handle_requests(&mut opt_resend_buffer);
                }
            }

            // If resend buffer is enabled, send any responses in it
            if let Some(resend_buffer) = opt_resend_buffer.as_mut() {
                for (addr, response) in resend_buffer.drain(..) {
                    self.send_response(&mut None, addr, response);
                }
            }
        }
    }

    fn read_and_handle_requests(
        &mut self,
        opt_resend_buffer: &mut Option<Vec<(CanonicalSocketAddr, Response)>>,
    ) {
        let max_scrape_torrents = self.config.protocol.max_scrape_torrents;

        loop {
            match self.socket.recv_from(&mut self.buffer[..]) {
                Ok((bytes_read, src)) => {
                    let src_port = src.port();
                    let src = CanonicalSocketAddr::new(src);

                    // Use canonical address for statistics
                    let opt_statistics = if self.config.statistics.active() {
                        if src.is_ipv4() {
                            let statistics = &self.statistics.ipv4;

                            statistics
                                .bytes_received
                                .fetch_add(bytes_read + EXTRA_PACKET_SIZE_IPV4, Ordering::Relaxed);

                            Some(statistics)
                        } else {
                            let statistics = &self.statistics.ipv6;

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

                    match Request::parse_bytes(&self.buffer[..bytes_read], max_scrape_torrents) {
                        Ok(request) => {
                            if let Some(statistics) = opt_statistics {
                                statistics.requests.fetch_add(1, Ordering::Relaxed);
                            }

                            if let Some(response) = self.handle_request(request, src) {
                                self.send_response(opt_resend_buffer, src, response);
                            }
                        }
                        Err(RequestParseError::Sendable {
                            connection_id,
                            transaction_id,
                            err,
                        }) if self.validator.connection_id_valid(src, connection_id) => {
                            let response = ErrorResponse {
                                transaction_id,
                                message: err.into(),
                            };

                            self.send_response(opt_resend_buffer, src, Response::Error(response));

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
                        let peer_valid_until = ValidUntil::new(
                            self.shared_state.server_start_instant,
                            self.config.cleaning.max_peer_age,
                        );

                        let response = self.shared_state.torrent_maps.announce(
                            &self.config,
                            &self.statistics_sender,
                            &mut self.rng,
                            &request,
                            src,
                            peer_valid_until,
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

    fn send_response(
        &mut self,
        opt_resend_buffer: &mut Option<Vec<(CanonicalSocketAddr, Response)>>,
        canonical_addr: CanonicalSocketAddr,
        response: Response,
    ) {
        let mut buffer = Cursor::new(&mut self.buffer[..]);

        if let Err(err) = response.write_bytes(&mut buffer) {
            ::log::error!("failed writing response to buffer: {:#}", err);

            return;
        }

        let bytes_written = buffer.position() as usize;

        let addr = if self.config.network.address.is_ipv4() {
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
            Ok(bytes_sent) if self.config.statistics.active() => {
                let stats = if canonical_addr.is_ipv4() {
                    let stats = &self.statistics.ipv4;

                    stats
                        .bytes_sent
                        .fetch_add(bytes_sent + EXTRA_PACKET_SIZE_IPV4, Ordering::Relaxed);

                    stats
                } else {
                    let stats = &self.statistics.ipv6;

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
            Err(err) => match opt_resend_buffer.as_mut() {
                Some(resend_buffer)
                    if (err.raw_os_error() == Some(libc::ENOBUFS))
                        || (err.kind() == ErrorKind::WouldBlock) =>
                {
                    if resend_buffer.len() < self.config.network.resend_buffer_max_len {
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
}
