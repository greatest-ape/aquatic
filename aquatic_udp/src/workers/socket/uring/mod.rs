mod buf_ring;
mod recv_helper;
mod send_buffers;

use std::collections::VecDeque;
use std::net::UdpSocket;
use std::os::fd::AsRawFd;

use aquatic_common::access_list::AccessListCache;
use aquatic_common::ServerStartInstant;
use crossbeam_channel::Receiver;
use io_uring::opcode::Timeout;
use io_uring::types::{Fixed, Timespec};
use io_uring::IoUring;

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

use super::create_socket;
use super::storage::PendingScrapeResponseSlab;
use super::validator::ConnectionValidator;

const RING_ENTRIES: u32 = 1024;
const SEND_ENTRIES: usize = 512;
const BUF_LEN: usize = 8192;

const USER_DATA_RECV: u64 = u64::MAX;
const USER_DATA_TIMEOUT: u64 = u64::MAX - 1;

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
            socket,
        };

        worker.run_inner();
    }

    pub fn run_inner(&mut self) {
        let mut local_responses: VecDeque<(Response, CanonicalSocketAddr)> = VecDeque::new();
        let mut pending_scrape_valid_until = ValidUntil::new(
            self.server_start_instant,
            self.config.cleaning.max_pending_scrape_age,
        );

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

        let timeout_timespec = Timespec::new().sec(1);

        let mut resubmit_recv = true;

        loop {
            let mut num_send_added = 0;
            let mut send_buffer_index = 0;

            let sq_space = {
                let mut sq = ring.submission();

                sq.sync();

                sq.capacity() - sq.len()
            };

            // Enqueue local responses
            for _ in 0..sq_space {
                if let Some((response, addr)) = local_responses.pop_front() {
                    match self
                        .send_buffers
                        .prepare_entry(send_buffer_index, response, addr)
                    {
                        Ok((index, entry)) => {
                            unsafe {
                                ring.submission().push(&entry).unwrap();
                            }

                            send_buffer_index = index + 1;
                            num_send_added += 1;
                        }
                        Err(send_buffers::Error::NoBuffers((response, addr))) => {
                            local_responses.push_front((response, addr));

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
                let mut sq = ring.submission();

                sq.sync();

                sq.capacity() - sq.len()
            };

            // Enqueue responses from swarm workers
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

                match self
                    .send_buffers
                    .prepare_entry(send_buffer_index, response, addr)
                {
                    Ok((index, entry)) => {
                        unsafe {
                            ring.submission().push(&entry).unwrap();
                        }

                        send_buffer_index = index + 1;
                        num_send_added += 1;
                    }
                    Err(send_buffers::Error::NoBuffers((response, addr))) => {
                        local_responses.push_back((response, addr));

                        break;
                    }
                    Err(send_buffers::Error::SerializationFailed(err)) => {
                        ::log::error!("write response to buffer: {:#}", err);
                    }
                }
            }

            unsafe {
                let entry = Timeout::new(&timeout_timespec as *const _)
                    .build()
                    .user_data(USER_DATA_TIMEOUT);

                ring.submission().push(&entry).unwrap();
            }

            ring.submitter()
                .submit_and_wait(num_send_added + 1)
                .unwrap();

            ring.completion().sync();

            let mut recv_in_cq = 0;
            let cq_len = ring.completion().len();

            for cqe in ring.completion() {
                match cqe.user_data() {
                    USER_DATA_RECV => {
                        self.handle_recv_cqe(
                            &buf_ring,
                            &mut local_responses,
                            pending_scrape_valid_until,
                            &cqe,
                        );

                        if !io_uring::cqueue::more(cqe.flags()) {
                            resubmit_recv = true;
                        }

                        recv_in_cq += 1;
                    }
                    USER_DATA_TIMEOUT => {}
                    send_buffer_index => {
                        self.send_buffers
                            .mark_index_as_free(send_buffer_index as usize);

                        let result = cqe.result();

                        if result < 0 {
                            ::log::error!(
                                "send: {:#}",
                                ::std::io::Error::from_raw_os_error(-result)
                            );
                        }
                    }
                }
            }

            println!(
                "num_send_added: {num_send_added}, cq_len: {cq_len}, recv_in_cq: {recv_in_cq}"
            );

            if resubmit_recv {
                let recv_msg_multi = self
                    .recv_helper
                    .create_entry(buf_ring.rc.bgid().try_into().unwrap());

                unsafe {
                    ring.submission().push(&recv_msg_multi).unwrap();
                }

                ring.submitter().submit().unwrap();

                resubmit_recv = false;
            }
        }
    }

    fn handle_recv_cqe(
        &mut self,
        buf_ring: &FixedSizeBufRing,
        local_responses: &mut VecDeque<(Response, CanonicalSocketAddr)>,
        pending_scrape_valid_until: ValidUntil,
        cqe: &io_uring::cqueue::Entry,
    ) {
        let result = cqe.result();

        if result < 0 {
            // Will produce ENOBUFS if there were no free buffers
            ::log::error!("recv: {:#}", ::std::io::Error::from_raw_os_error(-result));

            return;
        }

        let buffer = buf_ring
            .rc
            .get_buf(buf_ring.clone(), result as u32, cqe.flags())
            .unwrap();

        match self.recv_helper.parse(buffer.as_slice()) {
            (Ok(request), addr) => {
                self.handle_request(local_responses, pending_scrape_valid_until, request, addr)
            }
            (
                Err(RequestParseError::Sendable {
                    connection_id,
                    transaction_id,
                    err,
                }),
                addr,
            ) => {
                ::log::debug!("Couldn't parse request from {:?}: {}", addr, err);

                if self.validator.connection_id_valid(addr, connection_id) {
                    let response = ErrorResponse {
                        transaction_id,
                        message: err.right_or("Parse error").into(),
                    };

                    local_responses.push_back((response.into(), addr));
                }
            }
            (Err(RequestParseError::Unsendable { err }), addr) => {
                ::log::debug!("Couldn't parse request from {:?}: {}", addr, err);
            }
        }
    }

    fn handle_request(
        &mut self,
        local_responses: &mut VecDeque<(Response, CanonicalSocketAddr)>,
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

                local_responses.push_back((response, src))
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

                        local_responses.push_back((response, src))
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
