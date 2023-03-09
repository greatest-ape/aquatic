mod buf_ring;
mod recv_helper;
mod send_buffers;

use std::cell::RefCell;
use std::collections::VecDeque;
use std::net::UdpSocket;
use std::ops::DerefMut;
use std::os::fd::AsRawFd;
use std::sync::atomic::Ordering;

use anyhow::Context;
use aquatic_common::access_list::AccessListCache;
use aquatic_common::ServerStartInstant;
use crossbeam_channel::Receiver;
use io_uring::opcode::Timeout;
use io_uring::types::{Fixed, Timespec};
use io_uring::{IoUring, Probe};

use aquatic_common::{
    access_list::create_access_list_cache, privileges::PrivilegeDropper, CanonicalSocketAddr,
    PanicSentinel, ValidUntil,
};
use aquatic_udp_protocol::*;

use crate::common::*;
use crate::config::Config;

use self::buf_ring::BufRing;
use self::recv_helper::RecvHelper;
use self::send_buffers::{ResponseType, SendBuffers};

use super::storage::PendingScrapeResponseSlab;
use super::validator::ConnectionValidator;
use super::{create_socket, EXTRA_PACKET_SIZE_IPV4, EXTRA_PACKET_SIZE_IPV6};

const RESPONSE_BUF_LEN: usize = 8192;
const REQUEST_BUF_LEN: usize = 4096;

const USER_DATA_RECV: u64 = u64::MAX;
const USER_DATA_PULSE_TIMEOUT: u64 = u64::MAX - 1;
const USER_DATA_CLEANING_TIMEOUT: u64 = u64::MAX - 2;

const SOCKET_IDENTIFIER: Fixed = Fixed(0);

thread_local! {
    /// Store IoUring instance here so that it can be accessed in BufRing::drop
    pub static CURRENT_RING: CurrentRing = CurrentRing(RefCell::new(None));
}

pub struct CurrentRing(RefCell<Option<IoUring>>);

impl CurrentRing {
    fn with<F, T>(mut f: F) -> T
    where
        F: FnMut(&mut IoUring) -> T,
    {
        CURRENT_RING.with(|r| {
            let mut opt_ring = r.0.borrow_mut();

            f(Option::as_mut(opt_ring.deref_mut()).expect("IoUring not set"))
        })
    }
}

pub struct SocketWorker {
    config: Config,
    shared_state: State,
    request_sender: ConnectedRequestSender,
    response_receiver: Receiver<(ConnectedResponse, CanonicalSocketAddr)>,
    access_list_cache: AccessListCache,
    validator: ConnectionValidator,
    server_start_instant: ServerStartInstant,
    pending_scrape_responses: PendingScrapeResponseSlab,
    send_buffers: SendBuffers,
    recv_helper: RecvHelper,
    local_responses: VecDeque<(Response, CanonicalSocketAddr)>,
    pulse_timeout: Timespec,
    cleaning_timeout: Timespec,
    buf_ring: BufRing,
    #[allow(dead_code)]
    socket: UdpSocket,
}

impl SocketWorker {
    pub fn run(
        _sentinel: PanicSentinel,
        shared_state: State,
        config: Config,
        validator: ConnectionValidator,
        server_start_instant: ServerStartInstant,
        request_sender: ConnectedRequestSender,
        response_receiver: Receiver<(ConnectedResponse, CanonicalSocketAddr)>,
        priv_dropper: PrivilegeDropper,
    ) {
        let ring_entries = config.network.ring_entries.next_power_of_two();
        // Bias ring towards sending to prevent build-up of unsent responses
        let send_buffer_entries = ring_entries - (ring_entries / 4);

        let socket = create_socket(&config, priv_dropper).expect("create socket");
        let access_list_cache = create_access_list_cache(&shared_state.access_list);
        let send_buffers = SendBuffers::new(&config, send_buffer_entries as usize);
        let recv_helper = RecvHelper::new(&config);
        let cleaning_timeout =
            Timespec::new().sec(config.cleaning.pending_scrape_cleaning_interval);

        let ring = IoUring::builder()
            .setup_coop_taskrun()
            .setup_single_issuer()
            .setup_submit_all()
            .build(ring_entries.into())
            .unwrap();

        ring.submitter()
            .register_files(&[socket.as_raw_fd()])
            .unwrap();

        // Store ring in thread local storage before creating BufRing
        CURRENT_RING.with(|r| *r.0.borrow_mut() = Some(ring));

        let buf_ring = buf_ring::Builder::new(0)
            .ring_entries(ring_entries)
            .buf_len(REQUEST_BUF_LEN)
            .build()
            .unwrap();

        let mut worker = Self {
            config,
            shared_state,
            validator,
            server_start_instant,
            request_sender,
            response_receiver,
            access_list_cache,
            pending_scrape_responses: Default::default(),
            send_buffers,
            recv_helper,
            local_responses: Default::default(),
            pulse_timeout: Timespec::new().sec(1),
            cleaning_timeout,
            buf_ring,
            socket,
        };

        CurrentRing::with(|ring| worker.run_inner(ring));
    }

    pub fn run_inner(&mut self, ring: &mut IoUring) {
        let mut pending_scrape_valid_until = ValidUntil::new(
            self.server_start_instant,
            self.config.cleaning.max_pending_scrape_age,
        );

        let recv_entry = self
            .recv_helper
            .create_entry(self.buf_ring.bgid().try_into().unwrap());
        // This timeout enables regular updates of pending_scrape_valid_until
        // and wakes the main loop to send any pending responses in the case
        // of no incoming requests
        let pulse_timeout_entry = Timeout::new(&self.pulse_timeout as *const _)
            .build()
            .user_data(USER_DATA_PULSE_TIMEOUT);
        let cleaning_timeout_entry = Timeout::new(&self.cleaning_timeout as *const _)
            .build()
            .user_data(USER_DATA_CLEANING_TIMEOUT);

        let mut squeue_buf = vec![
            recv_entry.clone(),
            pulse_timeout_entry.clone(),
            cleaning_timeout_entry.clone(),
        ];

        loop {
            for sqe in squeue_buf.drain(..) {
                unsafe { ring.submission().push(&sqe).unwrap() };
            }

            let mut num_send_added = 0;

            let sq_space = {
                let sq = ring.submission();

                sq.capacity() - sq.len()
            };

            // Enqueue local responses
            for _ in 0..sq_space {
                if let Some((response, addr)) = self.local_responses.pop_front() {
                    match self.send_buffers.prepare_entry(&response, addr) {
                        Ok(entry) => {
                            unsafe { ring.submission().push(&entry).unwrap() };

                            num_send_added += 1;
                        }
                        Err(send_buffers::Error::NoBuffers) => {
                            self.local_responses.push_front((response, addr));

                            break;
                        }
                        Err(send_buffers::Error::SerializationFailed(err)) => {
                            ::log::error!("Failed serializing response: {:#}", err);
                        }
                    }
                } else {
                    break;
                }
            }

            let sq_space = {
                let sq = ring.submission();

                sq.capacity() - sq.len()
            };

            // Enqueue swarm worker responses
            'outer: for _ in 0..sq_space {
                let (response, addr) = loop {
                    match self.response_receiver.try_recv() {
                        Ok((ConnectedResponse::AnnounceIpv4(response), addr)) => {
                            break (Response::AnnounceIpv4(response), addr);
                        }
                        Ok((ConnectedResponse::AnnounceIpv6(response), addr)) => {
                            break (Response::AnnounceIpv6(response), addr);
                        }
                        Ok((ConnectedResponse::Scrape(response), addr)) => {
                            if let Some(response) =
                                self.pending_scrape_responses.add_and_get_finished(response)
                            {
                                break (Response::Scrape(response), addr);
                            }
                        }
                        Err(_) => {
                            break 'outer;
                        }
                    }
                };

                match self.send_buffers.prepare_entry(&response, addr) {
                    Ok(entry) => {
                        unsafe { ring.submission().push(&entry).unwrap() };

                        num_send_added += 1;
                    }
                    Err(send_buffers::Error::NoBuffers) => {
                        self.local_responses.push_back((response, addr));

                        break;
                    }
                    Err(send_buffers::Error::SerializationFailed(err)) => {
                        ::log::error!("Failed serializing response: {:#}", err);
                    }
                }
            }

            // Wait for all sendmsg entries to complete. If none were added,
            // wait for at least one recvmsg or timeout in order to avoid
            // busy-polling if there is no incoming data.
            ring.submitter()
                .submit_and_wait(num_send_added.max(1))
                .unwrap();

            for cqe in ring.completion() {
                match cqe.user_data() {
                    USER_DATA_RECV => {
                        self.handle_recv_cqe(pending_scrape_valid_until, &cqe);

                        if !io_uring::cqueue::more(cqe.flags()) {
                            squeue_buf.push(recv_entry.clone());
                        }
                    }
                    USER_DATA_PULSE_TIMEOUT => {
                        pending_scrape_valid_until = ValidUntil::new(
                            self.server_start_instant,
                            self.config.cleaning.max_pending_scrape_age,
                        );

                        ::log::info!(
                            "pending responses: {} local, {} swarm",
                            self.local_responses.len(),
                            self.response_receiver.len()
                        );

                        squeue_buf.push(pulse_timeout_entry.clone());
                    }
                    USER_DATA_CLEANING_TIMEOUT => {
                        self.pending_scrape_responses
                            .clean(self.server_start_instant.seconds_elapsed());

                        squeue_buf.push(cleaning_timeout_entry.clone());
                    }
                    send_buffer_index => {
                        let result = cqe.result();

                        if result < 0 {
                            ::log::error!(
                                "Couldn't send response: {:#}",
                                ::std::io::Error::from_raw_os_error(-result)
                            );
                        } else if self.config.statistics.active() {
                            let send_buffer_index = send_buffer_index as usize;

                            let (response_type, receiver_is_ipv4) =
                                self.send_buffers.response_type_and_ipv4(send_buffer_index);

                            let (statistics, extra_bytes) = if receiver_is_ipv4 {
                                (&self.shared_state.statistics_ipv4, EXTRA_PACKET_SIZE_IPV4)
                            } else {
                                (&self.shared_state.statistics_ipv6, EXTRA_PACKET_SIZE_IPV6)
                            };

                            statistics
                                .bytes_sent
                                .fetch_add(result as usize + extra_bytes, Ordering::Relaxed);

                            let response_counter = match response_type {
                                ResponseType::Connect => &statistics.responses_sent_connect,
                                ResponseType::Announce => &statistics.responses_sent_announce,
                                ResponseType::Scrape => &statistics.responses_sent_scrape,
                                ResponseType::Error => &statistics.responses_sent_error,
                            };

                            response_counter.fetch_add(1, Ordering::Relaxed);
                        }

                        // Safety: OK because cqe using buffer has been
                        // returned and contents will no longer be accessed
                        // by kernel
                        unsafe {
                            self.send_buffers
                                .mark_index_as_free(send_buffer_index as usize);
                        }
                    }
                }
            }

            self.send_buffers.reset_index();
        }
    }

    fn handle_recv_cqe(
        &mut self,
        pending_scrape_valid_until: ValidUntil,
        cqe: &io_uring::cqueue::Entry,
    ) {
        let result = cqe.result();

        if result < 0 {
            if -result == libc::ENOBUFS {
                ::log::warn!("recv failed due to lack of buffers, try increasing ring size");
            } else {
                ::log::warn!(
                    "recv failed: {:#}",
                    ::std::io::Error::from_raw_os_error(-result)
                );
            }

            return;
        }

        let buffer = unsafe {
            match self.buf_ring.get_buf(result as u32, cqe.flags()) {
                Ok(Some(buffer)) => buffer,
                Ok(None) => {
                    ::log::error!("Couldn't get recv buffer");

                    return;
                }
                Err(err) => {
                    ::log::error!("Couldn't get recv buffer: {:#}", err);

                    return;
                }
            }
        };

        let buffer = buffer.as_slice();

        let addr = match self.recv_helper.parse(buffer) {
            Ok((request, addr)) => {
                self.handle_request(pending_scrape_valid_until, request, addr);

                addr
            }
            Err(self::recv_helper::Error::RequestParseError(err, addr)) => {
                match err {
                    RequestParseError::Sendable {
                        connection_id,
                        transaction_id,
                        err,
                    } => {
                        ::log::debug!("Couldn't parse request from {:?}: {}", addr, err);

                        if self.validator.connection_id_valid(addr, connection_id) {
                            let response = ErrorResponse {
                                transaction_id,
                                message: err.right_or("Parse error").into(),
                            };

                            self.local_responses.push_back((response.into(), addr));
                        }
                    }
                    RequestParseError::Unsendable { err } => {
                        ::log::debug!("Couldn't parse request from {:?}: {}", addr, err);
                    }
                }

                addr
            }
            Err(self::recv_helper::Error::InvalidSocketAddress) => {
                ::log::debug!("Ignored request claiming to be from port 0");

                return;
            }
            Err(self::recv_helper::Error::RecvMsgParseError) => {
                ::log::error!("RecvMsgOut::parse failed");

                return;
            }
        };

        if self.config.statistics.active() {
            let (statistics, extra_bytes) = if addr.is_ipv4() {
                (&self.shared_state.statistics_ipv4, EXTRA_PACKET_SIZE_IPV4)
            } else {
                (&self.shared_state.statistics_ipv6, EXTRA_PACKET_SIZE_IPV6)
            };

            statistics
                .bytes_received
                .fetch_add(buffer.len() + extra_bytes, Ordering::Relaxed);
            statistics.requests_received.fetch_add(1, Ordering::Relaxed);
        }
    }

    fn handle_request(
        &mut self,
        pending_scrape_valid_until: ValidUntil,
        request: Request,
        src: CanonicalSocketAddr,
    ) {
        let access_list_mode = self.config.access_list.mode;

        match request {
            Request::Connect(request) => {
                let connection_id = self.validator.create_connection_id(src);

                let response = Response::Connect(ConnectResponse {
                    connection_id,
                    transaction_id: request.transaction_id,
                });

                self.local_responses.push_back((response, src));
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

                        self.request_sender.try_send_to(
                            worker_index,
                            ConnectedRequest::Announce(request),
                            src,
                        );
                    } else {
                        let response = Response::Error(ErrorResponse {
                            transaction_id: request.transaction_id,
                            message: "Info hash not allowed".into(),
                        });

                        self.local_responses.push_back((response, src))
                    }
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

                    for (swarm_worker_index, request) in split_requests {
                        self.request_sender.try_send_to(
                            swarm_worker_index,
                            ConnectedRequest::Scrape(request),
                            src,
                        );
                    }
                }
            }
        }
    }
}

pub fn supported_on_current_kernel() -> anyhow::Result<()> {
    let opcodes = [
        // We can't probe for RecvMsgMulti, so we probe for SendZc, which was
        // also introduced in Linux 6.0
        io_uring::opcode::SendZc::CODE,
    ];

    let ring = IoUring::new(1).with_context(|| "create ring")?;

    let mut probe = Probe::new();

    ring.submitter()
        .register_probe(&mut probe)
        .with_context(|| "register probe")?;

    for opcode in opcodes {
        if !probe.is_supported(opcode) {
            return Err(anyhow::anyhow!(
                "io_uring opcode {:b} not supported",
                opcode
            ));
        }
    }

    Ok(())
}
