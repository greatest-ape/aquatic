mod buf_ring;
mod recv_helper;
mod send_buffers;

use std::collections::VecDeque;
use std::net::UdpSocket;
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

use self::buf_ring::FixedSizeBufRing;
use self::recv_helper::RecvHelper;
use self::send_buffers::SendBuffers;

use super::storage::PendingScrapeResponseSlab;
use super::validator::ConnectionValidator;
use super::{create_socket, EXTRA_PACKET_SIZE_IPV4, EXTRA_PACKET_SIZE_IPV6};

const RING_ENTRIES: u32 = 1024;
const SEND_ENTRIES: usize = 512;
const BUF_LEN: usize = 8192;

const USER_DATA_RECV: u64 = u64::MAX;
const USER_DATA_PULSE_TIMEOUT: u64 = u64::MAX - 1;
const USER_DATA_CLEANING_TIMEOUT: u64 = u64::MAX - 2;

const SOCKET_IDENTIFIER: Fixed = Fixed(0);

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
        let socket = create_socket(&config, priv_dropper).expect("create socket");
        let access_list_cache = create_access_list_cache(&shared_state.access_list);
        let send_buffers = SendBuffers::new(&config, SEND_ENTRIES);
        let recv_helper = RecvHelper::new(&config);
        let cleaning_timeout =
            Timespec::new().sec(config.cleaning.pending_scrape_cleaning_interval);

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
            socket,
        };

        worker.run_inner();
    }

    pub fn run_inner(&mut self) {
        let mut ring = IoUring::builder()
            .setup_coop_taskrun()
            .setup_single_issuer()
            .build(RING_ENTRIES)
            .unwrap();

        ring.submitter()
            .register_files(&[self.socket.as_raw_fd()])
            .unwrap();

        let buf_ring = buf_ring::Builder::new(0)
            .ring_entries(RING_ENTRIES.try_into().unwrap())
            .buf_len(BUF_LEN)
            .build()
            .unwrap();

        buf_ring.rc.register(&mut ring).unwrap();

        let mut pending_scrape_valid_until = ValidUntil::new(
            self.server_start_instant,
            self.config.cleaning.max_pending_scrape_age,
        );

        let recv_entry = self
            .recv_helper
            .create_entry(buf_ring.rc.bgid().try_into().unwrap());
        // This timeout makes it possible to avoid busy-polling and enables
        // regular updates of pending_scrape_valid_until
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
            let mut bytes_sent_ipv4 = 0;
            let mut bytes_sent_ipv6 = 0;

            for sqe in squeue_buf.drain(..) {
                unsafe {
                    ring.submission().push(&sqe).unwrap();
                }
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
                            unsafe {
                                ring.submission().push(&entry).unwrap();
                            }

                            self.update_response_statistics(&response, addr);

                            num_send_added += 1;
                        }
                        Err(send_buffers::Error::NoBuffers) => {
                            self.local_responses.push_front((response, addr));

                            break;
                        }
                        Err(send_buffers::Error::SerializationFailed(err)) => {
                            ::log::error!("write response to buffer: {:#}", err);
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
                        unsafe {
                            ring.submission().push(&entry).unwrap();
                        }

                        self.update_response_statistics(&response, addr);

                        num_send_added += 1;
                    }
                    Err(send_buffers::Error::NoBuffers) => {
                        self.local_responses.push_back((response, addr));

                        break;
                    }
                    Err(send_buffers::Error::SerializationFailed(err)) => {
                        ::log::error!("write response to buffer: {:#}", err);
                    }
                }
            }

            // Wait for all sendmsg entries to complete, as well as at least
            // one recvmsg or timeout, in order to avoid busy-polling if there
            // is no incoming data.
            ring.submitter()
                .submit_and_wait(num_send_added + 1)
                .unwrap();

            for cqe in ring.completion() {
                match cqe.user_data() {
                    USER_DATA_RECV => {
                        self.handle_recv_cqe(&buf_ring, pending_scrape_valid_until, &cqe);

                        if !io_uring::cqueue::more(cqe.flags()) {
                            squeue_buf.push(recv_entry.clone());
                        }
                    }
                    USER_DATA_PULSE_TIMEOUT => {
                        pending_scrape_valid_until = ValidUntil::new(
                            self.server_start_instant,
                            self.config.cleaning.max_pending_scrape_age,
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
                                "send: {:#}",
                                ::std::io::Error::from_raw_os_error(-result)
                            );
                        } else if self.config.statistics.active() {
                            let bytes_sent = result as usize;

                            if self
                                .send_buffers
                                .receiver_is_ipv4(send_buffer_index as usize)
                            {
                                bytes_sent_ipv4 += bytes_sent + EXTRA_PACKET_SIZE_IPV4;
                            } else {
                                bytes_sent_ipv6 += bytes_sent + EXTRA_PACKET_SIZE_IPV6;
                            }
                        }

                        self.send_buffers
                            .mark_index_as_free(send_buffer_index as usize);
                    }
                }
            }

            self.send_buffers.reset_index();

            if self.config.statistics.active() {
                self.shared_state
                    .statistics_ipv4
                    .bytes_sent
                    .fetch_add(bytes_sent_ipv4, Ordering::Relaxed);
                self.shared_state
                    .statistics_ipv6
                    .bytes_sent
                    .fetch_add(bytes_sent_ipv6, Ordering::Relaxed);
            }
        }
    }

    fn handle_recv_cqe(
        &mut self,
        buf_ring: &FixedSizeBufRing,
        pending_scrape_valid_until: ValidUntil,
        cqe: &io_uring::cqueue::Entry,
    ) {
        let result = cqe.result();

        if result < 0 {
            // Will produce ENOBUFS if there were no free buffers
            ::log::warn!("recv: {:#}", ::std::io::Error::from_raw_os_error(-result));

            return;
        }

        let buffer = match buf_ring
            .rc
            .get_buf(buf_ring.clone(), result as u32, cqe.flags())
        {
            Ok(buffer) => buffer,
            Err(err) => {
                ::log::error!("Couldn't get buffer: {:#}", err);

                return;
            }
        };

        let buffer = buffer.as_slice();

        let (res_request, addr) = self.recv_helper.parse(buffer);

        match res_request {
            Ok(request) => self.handle_request(pending_scrape_valid_until, request, addr),
            Err(RequestParseError::Sendable {
                connection_id,
                transaction_id,
                err,
            }) => {
                ::log::debug!("Couldn't parse request from {:?}: {}", addr, err);

                if self.validator.connection_id_valid(addr, connection_id) {
                    let response = ErrorResponse {
                        transaction_id,
                        message: err.right_or("Parse error").into(),
                    };

                    self.local_responses.push_back((response.into(), addr));
                }
            }
            Err(RequestParseError::Unsendable { err }) => {
                ::log::debug!("Couldn't parse request from {:?}: {}", addr, err);
            }
        }

        if self.config.statistics.active() {
            if addr.is_ipv4() {
                self.shared_state
                    .statistics_ipv4
                    .bytes_received
                    .fetch_add(buffer.len() + EXTRA_PACKET_SIZE_IPV4, Ordering::Relaxed);
                self.shared_state
                    .statistics_ipv4
                    .requests_received
                    .fetch_add(1, Ordering::Relaxed);
            } else {
                self.shared_state
                    .statistics_ipv6
                    .bytes_received
                    .fetch_add(buffer.len() + EXTRA_PACKET_SIZE_IPV6, Ordering::Relaxed);
                self.shared_state
                    .statistics_ipv6
                    .requests_received
                    .fetch_add(1, Ordering::Relaxed);
            }
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

    fn update_response_statistics(&self, response: &Response, addr: CanonicalSocketAddr) {
        if self.config.statistics.active() {
            let statistics = if addr.is_ipv4() {
                &self.shared_state.statistics_ipv4
            } else {
                &self.shared_state.statistics_ipv6
            };

            match response {
                Response::Connect(_) => {
                    statistics
                        .responses_sent_connect
                        .fetch_add(1, Ordering::Relaxed);
                }
                Response::AnnounceIpv4(_) | Response::AnnounceIpv6(_) => {
                    statistics
                        .responses_sent_announce
                        .fetch_add(1, Ordering::Relaxed);
                }
                Response::Scrape(_) => {
                    statistics
                        .responses_sent_scrape
                        .fetch_add(1, Ordering::Relaxed);
                }
                Response::Error(_) => {
                    statistics
                        .responses_sent_error
                        .fetch_add(1, Ordering::Relaxed);
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
