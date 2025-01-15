mod buf_ring;
mod recv_helper;
mod send_buffers;

use std::cell::RefCell;
use std::collections::VecDeque;
use std::net::SocketAddr;
use std::net::UdpSocket;
use std::ops::DerefMut;
use std::os::fd::AsRawFd;
use std::sync::atomic::Ordering;

use anyhow::Context;
use aquatic_common::access_list::AccessListCache;
use crossbeam_channel::Sender;
use io_uring::opcode::Timeout;
use io_uring::types::{Fixed, Timespec};
use io_uring::{IoUring, Probe};
use recv_helper::RecvHelper;
use socket2::{Domain, Protocol, Socket, Type};

use aquatic_common::{
    access_list::create_access_list_cache, privileges::PrivilegeDropper, CanonicalSocketAddr,
    ValidUntil,
};
use aquatic_udp_protocol::*;
use rand::rngs::SmallRng;
use rand::SeedableRng;

use crate::common::*;
use crate::config::Config;

use self::buf_ring::BufRing;
use self::recv_helper::{RecvHelperV4, RecvHelperV6};
use self::send_buffers::{ResponseType, SendBuffers};

use super::validator::ConnectionValidator;
use super::{EXTRA_PACKET_SIZE_IPV4, EXTRA_PACKET_SIZE_IPV6};

/// Size of each request buffer
///
/// Needs to fit recvmsg metadata in addition to the payload.
///
/// The payload of a scrape request with 20 info hashes fits in 256 bytes.
const REQUEST_BUF_LEN: usize = 512;

/// Size of each response buffer
///
/// Enough for:
/// - IPv6 announce response with 112 peers
/// - scrape response for 170 info hashes
const RESPONSE_BUF_LEN: usize = 2048;

const USER_DATA_RECV_V4: u64 = u64::MAX;
const USER_DATA_RECV_V6: u64 = u64::MAX - 1;
const USER_DATA_PULSE_TIMEOUT: u64 = u64::MAX - 2;

const SOCKET_IDENTIFIER_V4: Fixed = Fixed(0);
const SOCKET_IDENTIFIER_V6: Fixed = Fixed(1);

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
    statistics: CachePaddedArc<IpVersionStatistics<SocketWorkerStatistics>>,
    statistics_sender: Sender<StatisticsMessage>,
    access_list_cache: AccessListCache,
    validator: ConnectionValidator,
    #[allow(dead_code)]
    opt_socket_ipv4: Option<UdpSocket>,
    #[allow(dead_code)]
    opt_socket_ipv6: Option<UdpSocket>,
    buf_ring: BufRing,
    send_buffers: SendBuffers,
    recv_helper_v4: RecvHelperV4,
    recv_helper_v6: RecvHelperV6,
    local_responses: VecDeque<(CanonicalSocketAddr, Response)>,
    resubmittable_sqe_buf: Vec<io_uring::squeue::Entry>,
    recv_sqe_ipv4: io_uring::squeue::Entry,
    recv_sqe_ipv6: io_uring::squeue::Entry,
    pulse_timeout_sqe: io_uring::squeue::Entry,
    peer_valid_until: ValidUntil,
    rng: SmallRng,
}

impl SocketWorker {
    pub fn run(
        config: Config,
        shared_state: State,
        statistics: CachePaddedArc<IpVersionStatistics<SocketWorkerStatistics>>,
        statistics_sender: Sender<StatisticsMessage>,
        validator: ConnectionValidator,
        mut priv_droppers: Vec<PrivilegeDropper>,
    ) -> anyhow::Result<()> {
        let ring_entries = config.network.ring_size.next_power_of_two();
        // Try to fill up the ring with send requests
        let send_buffer_entries = ring_entries;

        let opt_socket_ipv4 = if config.network.use_ipv4 {
            let priv_dropper = priv_droppers.pop().expect("not enough priv droppers");

            Some(
                create_socket(&config, priv_dropper, config.network.address_ipv4.into())
                    .context("create ipv4 socket")?,
            )
        } else {
            None
        };
        let opt_socket_ipv6 = if config.network.use_ipv6 {
            let priv_dropper = priv_droppers.pop().expect("not enough priv droppers");

            Some(
                create_socket(&config, priv_dropper, config.network.address_ipv6.into())
                    .context("create ipv6 socket")?,
            )
        } else {
            None
        };

        let access_list_cache = create_access_list_cache(&shared_state.access_list);

        let send_buffers = SendBuffers::new(send_buffer_entries as usize);
        let recv_helper_v4 = RecvHelperV4::new(&config);
        let recv_helper_v6 = RecvHelperV6::new(&config);

        let ring = IoUring::builder()
            .setup_coop_taskrun()
            .setup_single_issuer()
            .setup_submit_all()
            .build(ring_entries.into())
            .unwrap();

        ring.submitter()
            .register_files(&[
                opt_socket_ipv4
                    .as_ref()
                    .map(|s| s.as_raw_fd())
                    .unwrap_or(-1),
                opt_socket_ipv6
                    .as_ref()
                    .map(|s| s.as_raw_fd())
                    .unwrap_or(-1),
            ])
            .unwrap();

        // Store ring in thread local storage before creating BufRing
        CURRENT_RING.with(|r| *r.0.borrow_mut() = Some(ring));

        let buf_ring = buf_ring::Builder::new(0)
            .ring_entries(ring_entries)
            .buf_len(REQUEST_BUF_LEN)
            .build()
            .unwrap();

        // This timeout enables regular updates of ConnectionValidator and
        // peer_valid_until
        let pulse_timeout_sqe = {
            let timespec_ptr = Box::into_raw(Box::new(Timespec::new().sec(5))) as *const _;

            Timeout::new(timespec_ptr)
                .build()
                .user_data(USER_DATA_PULSE_TIMEOUT)
        };

        let mut resubmittable_sqe_buf = vec![pulse_timeout_sqe.clone()];

        let recv_sqe_ipv4 = recv_helper_v4.create_entry(buf_ring.bgid());
        let recv_sqe_ipv6 = recv_helper_v6.create_entry(buf_ring.bgid());

        if opt_socket_ipv4.is_some() {
            resubmittable_sqe_buf.push(recv_sqe_ipv4.clone());
        }
        if opt_socket_ipv6.is_some() {
            resubmittable_sqe_buf.push(recv_sqe_ipv6.clone());
        }

        let peer_valid_until = ValidUntil::new(
            shared_state.server_start_instant,
            config.cleaning.max_peer_age,
        );

        let mut worker = Self {
            config,
            shared_state,
            statistics,
            statistics_sender,
            validator,
            access_list_cache,
            opt_socket_ipv4,
            opt_socket_ipv6,
            send_buffers,
            recv_helper_v4,
            recv_helper_v6,
            local_responses: Default::default(),
            buf_ring,
            recv_sqe_ipv4,
            recv_sqe_ipv6,
            pulse_timeout_sqe,
            resubmittable_sqe_buf,
            peer_valid_until,
            rng: SmallRng::from_entropy(),
        };

        CurrentRing::with(|ring| worker.run_inner(ring));

        Ok(())
    }

    fn run_inner(&mut self, ring: &mut IoUring) {
        loop {
            for sqe in self.resubmittable_sqe_buf.drain(..) {
                unsafe { ring.submission().push(&sqe).unwrap() };
            }

            let sq_space = {
                let sq = ring.submission();

                sq.capacity() - sq.len()
            };

            let mut num_send_added = 0;

            // Enqueue local responses
            for _ in 0..sq_space {
                if let Some((addr, response)) = self.local_responses.pop_front() {
                    let send_to_ipv4_socket = if addr.is_ipv4() {
                        if self.opt_socket_ipv4.is_some() {
                            true
                        } else if self.opt_socket_ipv6.is_some() {
                            false
                        } else {
                            panic!("No socket open")
                        }
                    } else if self.opt_socket_ipv6.is_some() {
                        false
                    } else {
                        panic!("IPv6 response with no IPv6 socket")
                    };

                    match self
                        .send_buffers
                        .prepare_entry(send_to_ipv4_socket, response, addr)
                    {
                        Ok(entry) => {
                            unsafe { ring.submission().push(&entry).unwrap() };

                            num_send_added += 1;
                        }
                        Err(send_buffers::Error::NoBuffers(response)) => {
                            self.local_responses.push_front((addr, response));

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

            // Wait for all sendmsg entries to complete. If none were added,
            // wait for at least one recvmsg or timeout in order to avoid
            // busy-polling if there is no incoming data.
            ring.submitter()
                .submit_and_wait(num_send_added.max(1))
                .unwrap();

            for cqe in ring.completion() {
                self.handle_cqe(cqe);
            }

            self.send_buffers.reset_likely_next_free_index();
        }
    }

    fn handle_cqe(&mut self, cqe: io_uring::cqueue::Entry) {
        match cqe.user_data() {
            USER_DATA_RECV_V4 => {
                if let Some((addr, response)) = self.handle_recv_cqe(&cqe, true) {
                    self.local_responses.push_back((addr, response));
                }

                if !io_uring::cqueue::more(cqe.flags()) {
                    self.resubmittable_sqe_buf.push(self.recv_sqe_ipv4.clone());
                }
            }
            USER_DATA_RECV_V6 => {
                if let Some((addr, response)) = self.handle_recv_cqe(&cqe, false) {
                    self.local_responses.push_back((addr, response));
                }

                if !io_uring::cqueue::more(cqe.flags()) {
                    self.resubmittable_sqe_buf.push(self.recv_sqe_ipv6.clone());
                }
            }
            USER_DATA_PULSE_TIMEOUT => {
                self.validator.update_elapsed();

                self.peer_valid_until = ValidUntil::new(
                    self.shared_state.server_start_instant,
                    self.config.cleaning.max_peer_age,
                );

                self.resubmittable_sqe_buf
                    .push(self.pulse_timeout_sqe.clone());
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
                        (&self.statistics.ipv4, EXTRA_PACKET_SIZE_IPV4)
                    } else {
                        (&self.statistics.ipv6, EXTRA_PACKET_SIZE_IPV6)
                    };

                    statistics
                        .bytes_sent
                        .fetch_add(result as usize + extra_bytes, Ordering::Relaxed);

                    let response_counter = match response_type {
                        ResponseType::Connect => &statistics.responses_connect,
                        ResponseType::Announce => &statistics.responses_announce,
                        ResponseType::Scrape => &statistics.responses_scrape,
                        ResponseType::Error => &statistics.responses_error,
                    };

                    response_counter.fetch_add(1, Ordering::Relaxed);
                }

                // Safety: OK because cqe using buffer has been returned and
                // contents will no longer be accessed by kernel
                unsafe {
                    self.send_buffers
                        .mark_buffer_as_free(send_buffer_index as usize);
                }
            }
        }
    }

    fn handle_recv_cqe(
        &mut self,
        cqe: &io_uring::cqueue::Entry,
        received_on_ipv4_socket: bool,
    ) -> Option<(CanonicalSocketAddr, Response)> {
        let result = cqe.result();

        if result < 0 {
            if -result == libc::ENOBUFS {
                ::log::info!("recv failed due to lack of buffers, try increasing ring size");
            } else {
                ::log::warn!(
                    "recv failed: {:#}",
                    ::std::io::Error::from_raw_os_error(-result)
                );
            }

            return None;
        }

        let buffer = unsafe {
            match self.buf_ring.get_buf(result as u32, cqe.flags()) {
                Ok(Some(buffer)) => buffer,
                Ok(None) => {
                    ::log::error!("Couldn't get recv buffer");

                    return None;
                }
                Err(err) => {
                    ::log::error!("Couldn't get recv buffer: {:#}", err);

                    return None;
                }
            }
        };

        let recv_helper = if received_on_ipv4_socket {
            &self.recv_helper_v4 as &dyn RecvHelper
        } else {
            &self.recv_helper_v6 as &dyn RecvHelper
        };

        match recv_helper.parse(buffer.as_slice()) {
            Ok((request, addr)) => {
                if self.config.statistics.active() {
                    let (statistics, extra_bytes) = if addr.is_ipv4() {
                        (&self.statistics.ipv4, EXTRA_PACKET_SIZE_IPV4)
                    } else {
                        (&self.statistics.ipv6, EXTRA_PACKET_SIZE_IPV6)
                    };

                    statistics
                        .bytes_received
                        .fetch_add(buffer.len() + extra_bytes, Ordering::Relaxed);
                    statistics.requests.fetch_add(1, Ordering::Relaxed);
                }

                return self.handle_request(request, addr);
            }
            Err(self::recv_helper::Error::RequestParseError(err, addr)) => {
                if self.config.statistics.active() {
                    if addr.is_ipv4() {
                        self.statistics
                            .ipv4
                            .bytes_received
                            .fetch_add(buffer.len() + EXTRA_PACKET_SIZE_IPV4, Ordering::Relaxed);
                    } else {
                        self.statistics
                            .ipv6
                            .bytes_received
                            .fetch_add(buffer.len() + EXTRA_PACKET_SIZE_IPV6, Ordering::Relaxed);
                    }
                }

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
                                message: err.into(),
                            };

                            return Some((addr, Response::Error(response)));
                        }
                    }
                    RequestParseError::Unsendable { err } => {
                        ::log::debug!("Couldn't parse request from {:?}: {}", addr, err);
                    }
                }
            }
            Err(self::recv_helper::Error::InvalidSocketAddress) => {
                ::log::debug!("Ignored request claiming to be from port 0");
            }
            Err(self::recv_helper::Error::RecvMsgParseError) => {
                ::log::error!("RecvMsgOut::parse failed");
            }
            Err(self::recv_helper::Error::RecvMsgTruncated) => {
                ::log::warn!("RecvMsgOut::parse failed: sockaddr or payload truncated");
            }
        }

        None
    }

    fn handle_request(
        &mut self,
        request: Request,
        src: CanonicalSocketAddr,
    ) -> Option<(CanonicalSocketAddr, Response)> {
        let access_list_mode = self.config.access_list.mode;

        match request {
            Request::Connect(request) => {
                let response = Response::Connect(ConnectResponse {
                    connection_id: self.validator.create_connection_id(src),
                    transaction_id: request.transaction_id,
                });

                return Some((src, response));
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

                        return Some((src, response));
                    } else {
                        let response = Response::Error(ErrorResponse {
                            transaction_id: request.transaction_id,
                            message: "Info hash not allowed".into(),
                        });

                        return Some((src, response));
                    }
                }
            }
            Request::Scrape(request) => {
                if self
                    .validator
                    .connection_id_valid(src, request.connection_id)
                {
                    let response =
                        Response::Scrape(self.shared_state.torrent_maps.scrape(request, src));

                    return Some((src, response));
                }
            }
        }

        None
    }
}

fn create_socket(
    config: &Config,
    priv_dropper: PrivilegeDropper,
    address: SocketAddr,
) -> anyhow::Result<::std::net::UdpSocket> {
    let socket = if address.is_ipv4() {
        Socket::new(Domain::IPV4, Type::DGRAM, Some(Protocol::UDP))?
    } else {
        let socket = Socket::new(Domain::IPV6, Type::DGRAM, Some(Protocol::UDP))?;

        if config.network.set_only_ipv6 {
            socket
                .set_only_v6(true)
                .with_context(|| "socket: set only ipv6")?;
        }

        socket
    };

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
        .bind(&address.into())
        .with_context(|| format!("socket: bind to {}", address))?;

    priv_dropper.after_socket_creation()?;

    Ok(socket.into())
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
