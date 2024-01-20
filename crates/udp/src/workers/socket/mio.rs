use std::io::{Cursor, ErrorKind};
use std::sync::atomic::Ordering;
use std::time::{Duration, Instant};

use aquatic_common::access_list::AccessListCache;
use aquatic_common::ServerStartInstant;
use mio::net::UdpSocket;
use mio::{Events, Interest, Poll, Token};

use aquatic_common::{
    access_list::create_access_list_cache, privileges::PrivilegeDropper, CanonicalSocketAddr,
    PanicSentinel, ValidUntil,
};
use aquatic_udp_protocol::*;

use crate::common::*;
use crate::config::Config;

use super::storage::PendingScrapeResponseSlab;
use super::validator::ConnectionValidator;
use super::{create_socket, EXTRA_PACKET_SIZE_IPV4, EXTRA_PACKET_SIZE_IPV6};

enum HandleRequestError {
    RequestChannelFull(Vec<(SwarmWorkerIndex, ConnectedRequest, CanonicalSocketAddr)>),
}

#[derive(Clone, Copy, Debug)]
enum PollMode {
    Regular,
    SkipPolling,
    SkipReceiving,
}

pub struct SocketWorker {
    config: Config,
    shared_state: State,
    request_sender: ConnectedRequestSender,
    response_receiver: ConnectedResponseReceiver,
    access_list_cache: AccessListCache,
    validator: ConnectionValidator,
    server_start_instant: ServerStartInstant,
    pending_scrape_responses: PendingScrapeResponseSlab,
    socket: UdpSocket,
    opt_resend_buffer: Option<Vec<(CanonicalSocketAddr, Response)>>,
    buffer: [u8; BUFFER_SIZE],
    polling_mode: PollMode,
    /// Storage for requests that couldn't be sent to swarm worker because channel was full
    pending_requests: Vec<(SwarmWorkerIndex, ConnectedRequest, CanonicalSocketAddr)>,
}

impl SocketWorker {
    #[allow(clippy::too_many_arguments)]
    pub fn run(
        _sentinel: PanicSentinel,
        shared_state: State,
        config: Config,
        validator: ConnectionValidator,
        server_start_instant: ServerStartInstant,
        request_sender: ConnectedRequestSender,
        response_receiver: ConnectedResponseReceiver,
        priv_dropper: PrivilegeDropper,
    ) {
        let socket =
            UdpSocket::from_std(create_socket(&config, priv_dropper).expect("create socket"));
        let access_list_cache = create_access_list_cache(&shared_state.access_list);
        let opt_resend_buffer = (config.network.resend_buffer_max_len > 0).then_some(Vec::new());

        let mut worker = Self {
            config,
            shared_state,
            validator,
            server_start_instant,
            request_sender,
            response_receiver,
            access_list_cache,
            pending_scrape_responses: Default::default(),
            socket,
            opt_resend_buffer,
            buffer: [0; BUFFER_SIZE],
            polling_mode: PollMode::Regular,
            pending_requests: Default::default(),
        };

        worker.run_inner();
    }

    pub fn run_inner(&mut self) {
        let mut events = Events::with_capacity(1);
        let mut poll = Poll::new().expect("create poll");

        poll.registry()
            .register(&mut self.socket, Token(0), Interest::READABLE)
            .expect("register poll");

        let poll_timeout = Duration::from_millis(self.config.network.poll_timeout_ms);

        let pending_scrape_cleaning_duration =
            Duration::from_secs(self.config.cleaning.pending_scrape_cleaning_interval);

        let mut pending_scrape_valid_until = ValidUntil::new(
            self.server_start_instant,
            self.config.cleaning.max_pending_scrape_age,
        );
        let mut last_pending_scrape_cleaning = Instant::now();

        let mut iter_counter = 0usize;

        loop {
            match self.polling_mode {
                PollMode::Regular => {
                    poll.poll(&mut events, Some(poll_timeout))
                        .expect("failed polling");

                    for event in events.iter() {
                        if event.is_readable() {
                            self.read_and_handle_requests(pending_scrape_valid_until);
                        }
                    }
                }
                PollMode::SkipPolling => {
                    self.polling_mode = PollMode::Regular;

                    // Continue reading from socket without polling, since
                    // reading was previouly cancelled
                    self.read_and_handle_requests(pending_scrape_valid_until);
                }
                PollMode::SkipReceiving => {
                    ::log::debug!("Postponing receiving requests because swarm worker channel is full. This means that the OS will be relied on to buffer incoming packets. To prevent this, raise config.worker_channel_size.");

                    self.polling_mode = PollMode::SkipPolling;
                }
            }

            // If resend buffer is enabled, send any responses in it
            if let Some(resend_buffer) = self.opt_resend_buffer.as_mut() {
                for (addr, response) in resend_buffer.drain(..) {
                    Self::send_response(
                        &self.config,
                        &self.shared_state,
                        &mut self.socket,
                        &mut self.buffer,
                        &mut None,
                        response,
                        addr,
                    );
                }
            }

            // Check channel for any responses generated by swarm workers
            self.handle_swarm_worker_responses();

            // Try sending pending requests
            while let Some((index, request, addr)) = self.pending_requests.pop() {
                if let Err(r) = self.request_sender.try_send_to(index, request, addr) {
                    self.pending_requests.push(r);

                    self.polling_mode = PollMode::SkipReceiving;

                    break;
                }
            }

            // Run periodic ValidUntil updates and state cleaning
            if iter_counter % 256 == 0 {
                let seconds_since_start = self.server_start_instant.seconds_elapsed();

                pending_scrape_valid_until = ValidUntil::new_with_now(
                    seconds_since_start,
                    self.config.cleaning.max_pending_scrape_age,
                );

                let now = Instant::now();

                if now > last_pending_scrape_cleaning + pending_scrape_cleaning_duration {
                    self.pending_scrape_responses.clean(seconds_since_start);

                    last_pending_scrape_cleaning = now;
                }
            }

            iter_counter = iter_counter.wrapping_add(1);
        }
    }

    fn read_and_handle_requests(&mut self, pending_scrape_valid_until: ValidUntil) {
        let mut requests_received_ipv4: usize = 0;
        let mut requests_received_ipv6: usize = 0;
        let mut bytes_received_ipv4: usize = 0;
        let mut bytes_received_ipv6 = 0;

        loop {
            match self.socket.recv_from(&mut self.buffer[..]) {
                Ok((bytes_read, src)) => {
                    if src.port() == 0 {
                        ::log::debug!("Ignored request from {} because source port is zero", src);

                        continue;
                    }

                    let src = CanonicalSocketAddr::new(src);
                    let request_parsable = match Request::from_bytes(
                        &self.buffer[..bytes_read],
                        self.config.protocol.max_scrape_torrents,
                    ) {
                        Ok(request) => {
                            if let Err(HandleRequestError::RequestChannelFull(failed_requests)) =
                                self.handle_request(pending_scrape_valid_until, request, src)
                            {
                                self.pending_requests.extend(failed_requests);
                                self.polling_mode = PollMode::SkipReceiving;

                                break;
                            }

                            true
                        }
                        Err(err) => {
                            ::log::debug!("Request::from_bytes error: {:?}", err);

                            if let RequestParseError::Sendable {
                                connection_id,
                                transaction_id,
                                err,
                            } = err
                            {
                                if self.validator.connection_id_valid(src, connection_id) {
                                    let response = ErrorResponse {
                                        transaction_id,
                                        message: err.into(),
                                    };

                                    Self::send_response(
                                        &self.config,
                                        &self.shared_state,
                                        &mut self.socket,
                                        &mut self.buffer,
                                        &mut self.opt_resend_buffer,
                                        Response::Error(response),
                                        src,
                                    );
                                }
                            }

                            false
                        }
                    };

                    // Update statistics for converted address
                    if src.is_ipv4() {
                        if request_parsable {
                            requests_received_ipv4 += 1;
                        }
                        bytes_received_ipv4 += bytes_read + EXTRA_PACKET_SIZE_IPV4;
                    } else {
                        if request_parsable {
                            requests_received_ipv6 += 1;
                        }
                        bytes_received_ipv6 += bytes_read + EXTRA_PACKET_SIZE_IPV6;
                    }
                }
                Err(err) if err.kind() == ErrorKind::WouldBlock => {
                    break;
                }
                Err(err) => {
                    ::log::warn!("recv_from error: {:#}", err);
                }
            }
        }

        if self.config.statistics.active() {
            self.shared_state
                .statistics_ipv4
                .requests_received
                .fetch_add(requests_received_ipv4, Ordering::Relaxed);
            self.shared_state
                .statistics_ipv6
                .requests_received
                .fetch_add(requests_received_ipv6, Ordering::Relaxed);
            self.shared_state
                .statistics_ipv4
                .bytes_received
                .fetch_add(bytes_received_ipv4, Ordering::Relaxed);
            self.shared_state
                .statistics_ipv6
                .bytes_received
                .fetch_add(bytes_received_ipv6, Ordering::Relaxed);
        }
    }

    fn handle_request(
        &mut self,
        pending_scrape_valid_until: ValidUntil,
        request: Request,
        src: CanonicalSocketAddr,
    ) -> Result<(), HandleRequestError> {
        let access_list_mode = self.config.access_list.mode;

        match request {
            Request::Connect(request) => {
                let connection_id = self.validator.create_connection_id(src);

                let response = ConnectResponse {
                    connection_id,
                    transaction_id: request.transaction_id,
                };

                Self::send_response(
                    &self.config,
                    &self.shared_state,
                    &mut self.socket,
                    &mut self.buffer,
                    &mut self.opt_resend_buffer,
                    Response::Connect(response),
                    src,
                );

                Ok(())
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
                        let worker_index =
                            SwarmWorkerIndex::from_info_hash(&self.config, request.info_hash);

                        self.request_sender
                            .try_send_to(worker_index, ConnectedRequest::Announce(request), src)
                            .map_err(|request| {
                                HandleRequestError::RequestChannelFull(vec![request])
                            })
                    } else {
                        let response = ErrorResponse {
                            transaction_id: request.transaction_id,
                            message: "Info hash not allowed".into(),
                        };

                        Self::send_response(
                            &self.config,
                            &self.shared_state,
                            &mut self.socket,
                            &mut self.buffer,
                            &mut self.opt_resend_buffer,
                            Response::Error(response),
                            src,
                        );

                        Ok(())
                    }
                } else {
                    Ok(())
                }
            }
            Request::Scrape(request) => {
                if self
                    .validator
                    .connection_id_valid(src, request.connection_id)
                {
                    let split_requests = self.pending_scrape_responses.prepare_split_requests(
                        &self.config,
                        request,
                        pending_scrape_valid_until,
                    );

                    let mut failed = Vec::new();

                    for (swarm_worker_index, request) in split_requests {
                        if let Err(request) = self.request_sender.try_send_to(
                            swarm_worker_index,
                            ConnectedRequest::Scrape(request),
                            src,
                        ) {
                            failed.push(request);
                        }
                    }

                    if failed.is_empty() {
                        Ok(())
                    } else {
                        Err(HandleRequestError::RequestChannelFull(failed))
                    }
                } else {
                    Ok(())
                }
            }
        }
    }

    fn handle_swarm_worker_responses(&mut self) {
        for (addr, response) in self.response_receiver.try_iter() {
            let response = match response {
                ConnectedResponse::Scrape(response) => {
                    if let Some(r) = self
                        .pending_scrape_responses
                        .add_and_get_finished(&response)
                    {
                        Response::Scrape(r)
                    } else {
                        continue;
                    }
                }
                ConnectedResponse::AnnounceIpv4(r) => Response::AnnounceIpv4(r),
                ConnectedResponse::AnnounceIpv6(r) => Response::AnnounceIpv6(r),
            };

            Self::send_response(
                &self.config,
                &self.shared_state,
                &mut self.socket,
                &mut self.buffer,
                &mut self.opt_resend_buffer,
                response,
                addr,
            );
        }
    }

    fn send_response(
        config: &Config,
        shared_state: &State,
        socket: &mut UdpSocket,
        buffer: &mut [u8],
        opt_resend_buffer: &mut Option<Vec<(CanonicalSocketAddr, Response)>>,
        response: Response,
        canonical_addr: CanonicalSocketAddr,
    ) {
        let mut buffer = Cursor::new(&mut buffer[..]);

        if let Err(err) = response.write(&mut buffer) {
            ::log::error!("failed writing response to buffer: {:#}", err);

            return;
        }

        let bytes_written = buffer.position() as usize;

        let addr = if config.network.address.is_ipv4() {
            canonical_addr
                .get_ipv4()
                .expect("found peer ipv6 address while running bound to ipv4 address")
        } else {
            canonical_addr.get_ipv6_mapped()
        };

        match socket.send_to(&buffer.into_inner()[..bytes_written], addr) {
            Ok(amt) if config.statistics.active() => {
                let stats = if canonical_addr.is_ipv4() {
                    let stats = &shared_state.statistics_ipv4;

                    stats
                        .bytes_sent
                        .fetch_add(amt + EXTRA_PACKET_SIZE_IPV4, Ordering::Relaxed);

                    stats
                } else {
                    let stats = &shared_state.statistics_ipv6;

                    stats
                        .bytes_sent
                        .fetch_add(amt + EXTRA_PACKET_SIZE_IPV6, Ordering::Relaxed);

                    stats
                };

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
            Ok(_) => (),
            Err(err) => match opt_resend_buffer.as_mut() {
                Some(resend_buffer)
                    if (err.raw_os_error() == Some(libc::ENOBUFS))
                        || (err.kind() == ErrorKind::WouldBlock) =>
                {
                    if resend_buffer.len() < config.network.resend_buffer_max_len {
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
    }
}
