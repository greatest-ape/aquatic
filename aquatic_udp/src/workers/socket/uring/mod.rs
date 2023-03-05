mod buf_ring;

use std::collections::VecDeque;
use std::io::Cursor;
use std::net::{IpAddr, Ipv6Addr, SocketAddr, SocketAddrV4, SocketAddrV6};
use std::os::fd::AsRawFd;
use std::ptr::null_mut;

use anyhow::Context;
use aquatic_common::access_list::AccessListCache;
use aquatic_common::ServerStartInstant;
use crossbeam_channel::Receiver;
use io_uring::cqueue::more;
use io_uring::opcode::{RecvMsgMulti, SendMsg};
use io_uring::types::{Fixed, RecvMsgOut};
use io_uring::IoUring;
use libc::{c_void, msghdr};
use mio::net::UdpSocket;
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

const RING_ENTRIES: u32 = 1024;
const SEND_ENTRIES: usize = 512;
const BUF_LEN: usize = 8192;

struct SendBuffers {
    network_address: IpAddr,
    names_v4: Vec<libc::sockaddr_in>,
    names_v6: Vec<libc::sockaddr_in6>,
    buffers: Vec<[u8; BUF_LEN]>,
    iovecs: Vec<libc::iovec>,
    msghdrs: Vec<libc::msghdr>,
    free: Vec<bool>,
}

impl SendBuffers {
    fn new(config: &Config, capacity: usize) -> Self {
        let mut buffers = ::std::iter::repeat([0u8; BUF_LEN])
            .take(capacity)
            .collect::<Vec<_>>();

        let mut iovecs = buffers
            .iter_mut()
            .map(|buffer| libc::iovec {
                iov_base: buffer.as_mut_ptr() as *mut libc::c_void,
                iov_len: buffer.len(),
            })
            .collect::<Vec<_>>();

        let (names_v4, names_v6, msghdrs) = if config.network.address.is_ipv4() {
            let mut names_v4 = ::std::iter::repeat(libc::sockaddr_in {
                sin_family: 0,
                sin_port: 0,
                sin_addr: libc::in_addr { s_addr: 0 },
                sin_zero: [0; 8],
            })
            .take(capacity)
            .collect::<Vec<_>>();

            let msghdrs = names_v4
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

            (names_v4, Vec::new(), msghdrs)
        } else {
            let mut names_v6 = ::std::iter::repeat(libc::sockaddr_in6 {
                sin6_family: 0,
                sin6_port: 0,
                sin6_flowinfo: 0,
                sin6_addr: libc::in6_addr { s6_addr: [0; 16] },
                sin6_scope_id: 0,
            })
            .take(capacity)
            .collect::<Vec<_>>();

            let msghdrs = names_v6
                .iter_mut()
                .zip(iovecs.iter_mut())
                .map(|(msg_name, msg_iov)| libc::msghdr {
                    msg_name: msg_name as *mut _ as *mut c_void,
                    msg_namelen: core::mem::size_of::<libc::sockaddr_in6>() as u32,
                    msg_iov: msg_iov as *mut _,
                    msg_iovlen: 1,
                    msg_control: null_mut(),
                    msg_controllen: 0,
                    msg_flags: 0,
                })
                .collect::<Vec<_>>();

            (Vec::new(), names_v6, msghdrs)
        };

        Self {
            network_address: config.network.address.ip(),
            names_v4,
            names_v6,
            buffers,
            iovecs,
            msghdrs,
            free: ::std::iter::repeat(true).take(capacity).collect(),
        }
    }

    fn prepare_entry(
        &mut self,
        index: usize,
        response: Response,
        addr: CanonicalSocketAddr,
    ) -> Option<io_uring::squeue::Entry> {
        // Set receiver socket addr
        if self.network_address.is_ipv4() {
            let msg_name = self.names_v4.get_mut(index).unwrap();

            let addr = addr.get_ipv4().unwrap();

            msg_name.sin_port = addr.port().to_be();
            msg_name.sin_addr.s_addr = if let IpAddr::V4(addr) = addr.ip() {
                u32::from(addr).to_be()
            } else {
                panic!("ipv6 address in ipv4 mode");
            };
        } else {
            let msg_name = self.names_v6.get_mut(index).unwrap();

            let addr = addr.get_ipv6_mapped();

            msg_name.sin6_port = addr.port().to_be();
            msg_name.sin6_addr.s6_addr = if let IpAddr::V6(addr) = addr.ip() {
                addr.octets()
            } else {
                panic!("ipv4 address when ipv6 or ipv6-mapped address expected");
            };
        }

        let buf = self.buffers.get_mut(index).unwrap();
        let msg_hdr = self.msghdrs.get_mut(index).unwrap();
        let iov = self.iovecs.get_mut(index).unwrap();

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

    pub fn mark_index_as_free(&mut self, index: usize) {
        self.free[index] = true;
    }
}

struct RecvMsgMultiHelper {
    network_address: IpAddr,
    max_scrape_torrents: u8,
    #[allow(dead_code)]
    name_v4: Box<libc::sockaddr_in>,
    msghdr_v4: Box<libc::msghdr>,
    #[allow(dead_code)]
    name_v6: Box<libc::sockaddr_in6>,
    msghdr_v6: Box<libc::msghdr>,
}

impl RecvMsgMultiHelper {
    fn new(config: &Config) -> Self {
        let mut name_v4 = Box::new(libc::sockaddr_in {
            sin_family: 0,
            sin_port: 0,
            sin_addr: libc::in_addr { s_addr: 0 },
            sin_zero: [0; 8],
        });

        let msghdr_v4 = Box::new(msghdr {
            msg_name: &mut name_v4 as *mut _ as *mut libc::c_void,
            msg_namelen: core::mem::size_of::<libc::sockaddr_in>() as u32,
            msg_iov: null_mut(),
            msg_iovlen: 0,
            msg_control: null_mut(),
            msg_controllen: 0,
            msg_flags: 0,
        });

        let mut name_v6 = Box::new(libc::sockaddr_in6 {
            sin6_family: 0,
            sin6_port: 0,
            sin6_flowinfo: 0,
            sin6_addr: libc::in6_addr { s6_addr: [0; 16] },
            sin6_scope_id: 0,
        });

        let msghdr_v6 = Box::new(msghdr {
            msg_name: &mut name_v6 as *mut _ as *mut libc::c_void,
            msg_namelen: core::mem::size_of::<libc::sockaddr_in6>() as u32,
            msg_iov: null_mut(),
            msg_iovlen: 0,
            msg_control: null_mut(),
            msg_controllen: 0,
            msg_flags: 0,
        });

        Self {
            network_address: config.network.address.ip(),
            max_scrape_torrents: config.protocol.max_scrape_torrents,
            name_v4,
            msghdr_v4,
            name_v6,
            msghdr_v6,
        }
    }

    pub fn create_entry(&self, buf_group: u16) -> io_uring::squeue::Entry {
        let msghdr: *const libc::msghdr = if self.network_address.is_ipv4() {
            &*self.msghdr_v4
        } else {
            &*self.msghdr_v6
        };

        RecvMsgMulti::new(Fixed(0), msghdr, buf_group)
            .build()
            .user_data(u64::MAX)
    }

    pub fn parse(
        &self,
        buffer: &[u8],
    ) -> (Result<Request, RequestParseError>, CanonicalSocketAddr) {
        let msghdr = if self.network_address.is_ipv4() {
            &self.msghdr_v4
        } else {
            &self.msghdr_v6
        };

        let msg = RecvMsgOut::parse(buffer, msghdr).unwrap();

        let addr = unsafe {
            if self.network_address.is_ipv4() {
                let name_data = *(msg.name_data().as_ptr() as *const libc::sockaddr_in);

                SocketAddr::V4(SocketAddrV4::new(
                    u32::from_be(name_data.sin_addr.s_addr).into(),
                    u16::from_be(name_data.sin_port),
                ))
            } else {
                let name_data = *(msg.name_data().as_ptr() as *const libc::sockaddr_in6);

                SocketAddr::V6(SocketAddrV6::new(
                    Ipv6Addr::from(name_data.sin6_addr.s6_addr),
                    u16::from_be(name_data.sin6_port),
                    u32::from_be(name_data.sin6_flowinfo),
                    u32::from_be(name_data.sin6_scope_id),
                ))
            }
        };

        (
            Request::from_bytes(msg.payload_data(), self.max_scrape_torrents),
            CanonicalSocketAddr::new(addr),
        )
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

        let mut send_buffers = SendBuffers::new(&self.config, SEND_ENTRIES);
        let recv_msg_helper = RecvMsgMultiHelper::new(&self.config);

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
            'outer: for _ in 0..sq_space {
                // Find next free index
                loop {
                    match send_buffers.free.get(send_buffer_index) {
                        Some(true) => {
                            break;
                        }
                        Some(false) => {
                            send_buffer_index += 1;
                        }
                        None => {
                            break 'outer;
                        }
                    }
                }

                if let Some((response, addr)) = local_responses.pop_front() {
                    if let Some(entry) =
                        send_buffers.prepare_entry(send_buffer_index, response, addr)
                    {
                        unsafe {
                            ring.submission().push(&entry).unwrap();
                        }

                        num_send_added += 1;
                    }

                    send_buffer_index += 1;
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
                // Find next free index
                loop {
                    match send_buffers.free.get(send_buffer_index) {
                        Some(true) => {
                            break;
                        }
                        Some(false) => {
                            send_buffer_index += 1;
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
                        if let Some(entry) =
                            send_buffers.prepare_entry(send_buffer_index, response, addr)
                        {
                            unsafe {
                                ring.submission().push(&entry).unwrap();
                            }

                            num_send_added += 1;
                        }

                        send_buffer_index += 1;
                    }
                } else {
                    break;
                }
            }

            ring.submitter().submit_and_wait(num_send_added).unwrap();

            ring.completion().sync();

            let mut recv_in_cq = 0;
            let cq_len = ring.completion().len();

            for cqe in ring.completion() {
                if cqe.user_data() == u64::MAX {
                    recv_in_cq += 1;

                    let result = cqe.result();

                    if result < 0 {
                        // Expect ENOBUFS when the buf_ring is empty.
                        ::log::error!("recv: {:#}", ::std::io::Error::from_raw_os_error(-result));
                    } else {
                        let buffer = buf_ring
                            .rc
                            .get_buf(buf_ring.clone(), result as u32, cqe.flags())
                            .unwrap();

                        match recv_msg_helper.parse(buffer.as_slice()) {
                            (Ok(request), addr) => self.handle_request(
                                &mut local_responses,
                                pending_scrape_valid_until,
                                request,
                                addr,
                            ),
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

                    if !more(cqe.flags()) {
                        resubmit_recv = true;
                    }
                } else {
                    send_buffers.mark_index_as_free(cqe.user_data() as usize);

                    let result = cqe.result();

                    if result < 0 {
                        panic!("send: {:#}", ::std::io::Error::from_raw_os_error(-result));
                    }
                }
            }

            println!(
                "num_send_added: {num_send_added}, cq_len: {cq_len}, recv_in_cq: {recv_in_cq}"
            );

            if resubmit_recv {
                let recv_msg_multi =
                    recv_msg_helper.create_entry(buf_ring.rc.bgid().try_into().unwrap());

                unsafe {
                    ring.submission().push(&recv_msg_multi).unwrap();
                }

                ring.submitter().submit().unwrap();

                resubmit_recv = false;
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
