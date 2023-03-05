use std::io::{Cursor, ErrorKind};
use std::net::{IpAddr, Ipv4Addr, SocketAddrV4};
use std::os::fd::AsRawFd;
use std::ptr::{null, null_mut};
use std::rc::Rc;
use std::sync::atomic::Ordering;
use std::time::{Duration, Instant};

use anyhow::Context;
use aquatic_common::access_list::AccessListCache;
use aquatic_common::ServerStartInstant;
use crossbeam_channel::Receiver;
use io_uring::cqueue::{buffer_select, more};
use io_uring::opcode::{RecvMsgMulti, SendMsg};
use io_uring::types::{Fixed, RecvMsgOut};
use io_uring::IoUring;
use libc::{c_void, msghdr, sockaddr_in};
use mio::net::UdpSocket;
use mio::{Events, Interest, Poll, Token};
use socket2::{Domain, Protocol, Socket, Type};

use aquatic_common::{
    access_list::create_access_list_cache, privileges::PrivilegeDropper, CanonicalSocketAddr,
    PanicSentinel, ValidUntil,
};
use aquatic_udp_protocol::*;

use crate::common::*;
use crate::config::Config;

use super::storage::PendingScrapeResponseSlab;
use super::validator::ConnectionValidator;

struct OutMessageStorage {
    pub names: Vec<libc::sockaddr_in>,
    pub buffers: Vec<[u8; 8192]>,
    pub iovecs: Vec<libc::iovec>,
    pub msghdrs: Vec<libc::msghdr>,
    pub free: Vec<bool>,
}

impl OutMessageStorage {
    fn new(capacity: usize) -> Self {
        let mut names = ::std::iter::repeat(libc::sockaddr_in {
            sin_family: 0,
            sin_port: 0,
            sin_addr: libc::in_addr { s_addr: 0 },
            sin_zero: [0; 8],
        })
        .take(capacity)
        .collect::<Vec<_>>();

        let mut buffers = ::std::iter::repeat([0u8; 8192])
            .take(capacity)
            .collect::<Vec<_>>();

        let mut iovecs = buffers
            .iter_mut()
            .map(|buffer| libc::iovec {
                iov_base: buffer.as_mut_ptr() as *mut libc::c_void,
                iov_len: buffer.len(),
            })
            .collect::<Vec<_>>();

        let msghdrs = names
            .iter_mut()
            .zip(iovecs.iter_mut())
            .map(|(msg_name, msg_iov)| libc::msghdr {
                msg_name: msg_name as *mut _ as *mut c_void,
                msg_namelen: core::mem::size_of::<libc::sockaddr_in>() as u32,
                msg_iov: msg_iov as *mut _,
                msg_iovlen: 1,
                msg_control: null_mut(),
                msg_controllen: 0,
                msg_flags: 0,
            })
            .collect::<Vec<_>>();

        Self {
            names,
            buffers,
            iovecs,
            msghdrs,
            free: ::std::iter::repeat(true).take(capacity).collect(),
        }
    }

    fn add_entry(
        &mut self,
        index: usize,
        response: Response,
        addr: CanonicalSocketAddr,
    ) -> Option<io_uring::squeue::Entry> {
        let msg_hdr = self.msghdrs.get_mut(index).unwrap();
        let msg_name = self.names.get_mut(index).unwrap();
        let buf = self.buffers.get_mut(index).unwrap();
        let iov = self.iovecs.get_mut(index).unwrap();

        let addr = addr.get_ipv4().unwrap();

        msg_name.sin_port = addr.port().to_be();
        msg_name.sin_addr.s_addr = if let IpAddr::V4(addr) = addr.ip() {
            u32::from(addr).to_be()
        } else {
            panic!("ipv6");
        };

        let mut cursor = Cursor::new(buf.as_mut_slice());

        match response.write(&mut cursor) {
            Ok(()) => {
                iov.iov_len = cursor.position() as usize;
                *self.free.get_mut(index).unwrap() = false;

                Some(
                    SendMsg::new(Fixed(0), msg_hdr)
                        .build()
                        .user_data(index as u64),
                )
            }
            Err(err) => {
                ::log::error!("Converting response to bytes failed: {:#}", err);

                None
            }
        }
    }

    fn mark_index_as_free(&mut self, index: usize) {
        self.free[index] = true;
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
    socket: UdpSocket,
    buffer: [u8; BUFFER_SIZE],
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
        let socket =
            UdpSocket::from_std(create_socket(&config, priv_dropper).expect("create socket"));
        let access_list_cache = create_access_list_cache(&shared_state.access_list);

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
            buffer: [0; BUFFER_SIZE],
        };

        worker.run_inner();
    }

    pub fn run_inner(&mut self) {
        let mut local_responses: Vec<(Response, CanonicalSocketAddr)> = Vec::new();
        let mut pending_scrape_valid_until = ValidUntil::new(
            self.server_start_instant,
            self.config.cleaning.max_pending_scrape_age,
        );

        let mut ring = IoUring::new(128).expect("create uring");

        ring.submitter()
            .register_files(&[self.socket.as_raw_fd()])
            .unwrap();

        let buf_ring = super::buf_ring::Builder::new(0)
            .ring_entries(128)
            .buf_len(8192)
            .build()
            .unwrap();

        buf_ring.rc.register(&mut ring).unwrap();

        let mut recv_msg_multi_name = libc::sockaddr_in {
            sin_family: 0,
            sin_port: 0,
            sin_addr: libc::in_addr { s_addr: 0 },
            sin_zero: [0; 8],
        };

        let recv_msg_multi_msghdr = msghdr {
            msg_name: &mut recv_msg_multi_name as *mut _ as *mut libc::c_void,
            msg_namelen: core::mem::size_of::<libc::sockaddr_in>() as u32,
            msg_iov: null_mut(),
            msg_iovlen: 0,
            msg_control: null_mut(),
            msg_controllen: 0,
            msg_flags: 0,
        };

        let recv_msg_multi = RecvMsgMulti::new(
            Fixed(0),
            &recv_msg_multi_msghdr as *const _,
            buf_ring.rc.bgid(),
        )
        .build()
        .user_data(u64::MAX);

        let mut out = OutMessageStorage::new(64);

        let mut resubmit_recv = true;

        loop {
            let mut num_out_added = 0;

            let mut out_index = 0;

            // Enqueue local responses
            'outer: loop {
                // Find next free index
                loop {
                    match out.free.get(out_index) {
                        Some(true) => {
                            break;
                        }
                        Some(false) => {
                            out_index += 1;
                        }
                        None => {
                            break 'outer;
                        }
                    }
                }

                if let Some((response, addr)) = local_responses.pop() {
                    if let Some(entry) = out.add_entry(out_index, response, addr) {
                        unsafe {
                            ring.submission().push(&entry).unwrap();
                        }

                        num_out_added += 1;
                    }

                    out_index += 1;
                } else {
                    break;
                }
            }

            // Enqueue responses from swarm workers
            'outer: loop {
                // Find next free index
                loop {
                    match out.free.get(out_index) {
                        Some(true) => {
                            break;
                        }
                        Some(false) => {
                            out_index += 1;
                        }
                        None => {
                            break 'outer;
                        }
                    }
                }

                if let Ok((response, addr)) = self.response_receiver.try_recv() {
                    let opt_response = match response {
                        ConnectedResponse::Scrape(r) => self
                            .pending_scrape_responses
                            .add_and_get_finished(r)
                            .map(Response::Scrape),
                        ConnectedResponse::AnnounceIpv4(r) => Some(Response::AnnounceIpv4(r)),
                        ConnectedResponse::AnnounceIpv6(r) => Some(Response::AnnounceIpv6(r)),
                    };

                    if let Some(response) = opt_response {
                        if let Some(entry) = out.add_entry(out_index, response, addr) {
                            unsafe {
                                ring.submission().push(&entry).unwrap();
                            }

                            num_out_added += 1;
                        }

                        out_index += 1;
                    }
                } else {
                    break;
                }
            }

            ring.submitter().submit_and_wait(num_out_added).unwrap();

            for cqe in ring.completion() {
                if cqe.user_data() == u64::MAX {
                    let result = cqe.result();

                    if result < 0 {
                        // Expect ENOBUFS when the buf_ring is empty.
                        ::log::error!("recv: {:#}", ::std::io::Error::from_raw_os_error(-result));
                    } else {
                        let result = result as u32;

                        let buffer = buf_ring
                            .rc
                            .get_buf(buf_ring.clone(), result, cqe.flags())
                            .unwrap();
                        let msg =
                            RecvMsgOut::parse(buffer.as_slice(), &recv_msg_multi_msghdr).unwrap();

                        let addr = unsafe {
                            let name_data = *(msg.name_data().as_ptr() as *const libc::sockaddr_in);

                            SocketAddrV4::new(
                                u32::from_be(name_data.sin_addr.s_addr).into(),
                                u16::from_be(name_data.sin_port),
                            )
                        };

                        match Request::from_bytes(
                            msg.payload_data(),
                            self.config.protocol.max_scrape_torrents,
                        ) {
                            Ok(request) => self.handle_request(
                                &mut local_responses,
                                pending_scrape_valid_until,
                                request,
                                CanonicalSocketAddr::new(addr.into()),
                            ),
                            Err(err) => {}
                        }
                    }

                    if !more(cqe.flags()) {
                        resubmit_recv = true;
                    }
                } else {
                    out.mark_index_as_free(cqe.user_data() as usize);

                    let result = cqe.result();

                    if result < 0 {
                        panic!("send: {:#}", ::std::io::Error::from_raw_os_error(-result));
                    }
                }
            }

            if resubmit_recv {
                unsafe {
                    ring.submission().push(&recv_msg_multi).unwrap();
                }

                ring.submitter().submit_and_wait(1).unwrap();

                resubmit_recv = false;
            }
        }
    }

    fn handle_request(
        &mut self,
        local_responses: &mut Vec<(Response, CanonicalSocketAddr)>,
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

                local_responses.push((response, src))
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

                        local_responses.push((response, src))
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

fn create_socket(
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
