use std::mem::size_of;
use std::net::{Ipv4Addr, Ipv6Addr, SocketAddr, SocketAddrV4};
use std::os::unix::prelude::AsRawFd;
use std::ptr::null_mut;
use std::sync::{
    atomic::{AtomicUsize, Ordering},
    Arc,
};
use std::time::{Duration, Instant};

use aquatic_common::access_list::create_access_list_cache;
use aquatic_common::ValidUntil;
use crossbeam_channel::{Receiver, Sender};
use io_uring::types::{Fixed, Timespec};
use io_uring::SubmissionQueue;
use libc::{
    c_void, in6_addr, in_addr, iovec, msghdr, sockaddr_in, sockaddr_in6, AF_INET, AF_INET6,
};
use rand::prelude::{SeedableRng, StdRng};
use slab::Slab;

use aquatic_udp_protocol::*;

use crate::common::network::ConnectionMap;
use crate::common::network::*;
use crate::common::*;
use crate::config::Config;

const RING_SIZE: usize = 128;
const MAX_RECV_ENTRIES: usize = 1;
const MAX_SEND_ENTRIES: usize = RING_SIZE - MAX_RECV_ENTRIES - 1;

#[derive(Clone, Copy, Debug, PartialEq)]
enum UserData {
    RecvMsg { slab_key: usize },
    SendMsg { slab_key: usize },
    Timeout,
}

impl UserData {
    fn get_buffer_index(&self) -> usize {
        match self {
            Self::RecvMsg { slab_key } => *slab_key,
            Self::SendMsg { slab_key } => *slab_key,
            Self::Timeout => {
                unreachable!()
            }
        }
    }
}

impl From<u64> for UserData {
    fn from(mut n: u64) -> UserData {
        let bytes = bytemuck::bytes_of_mut(&mut n);

        let t = bytes[7];

        bytes[7] = 0;

        match t {
            0 => Self::RecvMsg {
                slab_key: n as usize,
            },
            1 => Self::SendMsg {
                slab_key: n as usize,
            },
            2 => Self::Timeout,
            _ => unreachable!(),
        }
    }
}

impl Into<u64> for UserData {
    fn into(self) -> u64 {
        match self {
            Self::RecvMsg { slab_key } => {
                let mut out = slab_key as u64;

                bytemuck::bytes_of_mut(&mut out)[7] = 0;

                out
            }
            Self::SendMsg { slab_key } => {
                let mut out = slab_key as u64;

                bytemuck::bytes_of_mut(&mut out)[7] = 1;

                out
            }
            Self::Timeout => {
                let mut out = 0u64;

                bytemuck::bytes_of_mut(&mut out)[7] = 2;

                out
            }
        }
    }
}

pub fn run_socket_worker(
    state: State,
    config: Config,
    request_sender: Sender<(ConnectedRequest, SocketAddr)>,
    response_receiver: Receiver<(ConnectedResponse, SocketAddr)>,
    num_bound_sockets: Arc<AtomicUsize>,
) {
    let mut rng = StdRng::from_entropy();

    let socket = create_socket(&config);

    num_bound_sockets.fetch_add(1, Ordering::SeqCst);

    let mut connections = ConnectionMap::default();
    let mut access_list_cache = create_access_list_cache(&state.access_list);
    let mut local_responses: Vec<(Response, SocketAddr)> = Vec::new();

    // Setup recv buffers

    let mut recv_buffers: Vec<[u8; MAX_PACKET_SIZE]> =
        (0..MAX_RECV_ENTRIES).map(|_| [0; MAX_PACKET_SIZE]).collect();

    let mut recv_sockaddrs_ipv4 = [sockaddr_in {
        sin_addr: in_addr { s_addr: 0 },
        sin_port: 0,
        sin_family: AF_INET as u16,
        sin_zero: Default::default(),
    }; MAX_RECV_ENTRIES];

    let mut recv_sockaddrs_ipv6 = [sockaddr_in6 {
        sin6_addr: in6_addr { s6_addr: [0; 16] },
        sin6_port: 0,
        sin6_family: AF_INET6 as u16,
        sin6_flowinfo: 0,
        sin6_scope_id: 0,
    }; MAX_RECV_ENTRIES];

    let mut recv_iovs: Vec<iovec> = (0..MAX_RECV_ENTRIES)
        .map(|i| {
            let iov_base = recv_buffers[i].as_mut_ptr() as *mut c_void;
            let iov_len = MAX_PACKET_SIZE;

            iovec { iov_base, iov_len }
        })
        .collect();

    let mut recv_msghdrs: Vec<msghdr> = (0..MAX_RECV_ENTRIES)
        .map(|i| {
            let msg_iov: *mut iovec = &mut recv_iovs[i];

            let mut msghdr = msghdr {
                msg_name: null_mut(),
                msg_namelen: 0,
                msg_iov,
                msg_iovlen: 1,
                msg_control: null_mut(),
                msg_controllen: 0,
                msg_flags: 0,
            };

            if config.network.address.is_ipv4() {
                let ptr: *mut sockaddr_in = &mut recv_sockaddrs_ipv4[i];

                msghdr.msg_name = ptr as *mut c_void;
                msghdr.msg_namelen = size_of::<sockaddr_in>() as u32;
            } else {
                let ptr: *mut sockaddr_in6 = &mut recv_sockaddrs_ipv6[i];

                msghdr.msg_name = ptr as *mut c_void;
                msghdr.msg_namelen = size_of::<sockaddr_in6>() as u32;
            }

            msghdr
        })
        .collect();

    // Setup send buffers

    let mut send_buffers_connect: Vec<ConnectResponse> =
        (0..MAX_SEND_ENTRIES).map(|_| ConnectResponse::new_zeroed()).collect();

    let mut send_buffers_announce_ipv4: Vec<AnnounceResponseIpv4> =
        (0..MAX_SEND_ENTRIES).map(|_| AnnounceResponseIpv4::new_zeroed()).collect();

    let mut send_buffers_announce_ipv6: Vec<AnnounceResponseIpv6> =
        (0..MAX_SEND_ENTRIES).map(|_| AnnounceResponseIpv6::new_zeroed()).collect();

    let mut send_buffers_scrape: Vec<ScrapeResponse> =
        (0..MAX_SEND_ENTRIES).map(|_| ScrapeResponse::new_zeroed()).collect();

    let mut send_buffers_error: Vec<ErrorResponse> =
        (0..MAX_SEND_ENTRIES).map(|_| ErrorResponse::new_zeroed()).collect();

    let mut send_sockaddrs_ipv4 = [sockaddr_in {
        sin_addr: in_addr { s_addr: 0 },
        sin_port: 0,
        sin_family: AF_INET as u16,
        sin_zero: Default::default(),
    }; MAX_SEND_ENTRIES];

    let mut send_sockaddrs_ipv6 = [sockaddr_in6 {
        sin6_addr: in6_addr { s6_addr: [0; 16] },
        sin6_port: 0,
        sin6_family: AF_INET6 as u16,
        sin6_flowinfo: 0,
        sin6_scope_id: 0,
    }; MAX_SEND_ENTRIES];

    let mut send_iovs: Vec<[iovec; 2]> = (0..MAX_SEND_ENTRIES)
        .map(|_| {
            [
                iovec { iov_base: null_mut(), iov_len: 0 },
                iovec { iov_base: null_mut(), iov_len: 0 },
            ]
        })
        .collect();

    let mut send_msghdrs: Vec<msghdr> = (0..MAX_SEND_ENTRIES)
        .map(|i| {
            let msg_iov: *mut iovec = &mut send_iovs[i][0];

            let mut msghdr = msghdr {
                msg_name: null_mut(),
                msg_namelen: 0,
                msg_iov,
                msg_iovlen: 2,
                msg_control: null_mut(),
                msg_controllen: 0,
                msg_flags: 0,
            };

            if config.network.address.is_ipv4() {
                let ptr: *mut sockaddr_in = &mut send_sockaddrs_ipv4[i];

                msghdr.msg_name = ptr as *mut c_void;
                msghdr.msg_namelen = size_of::<sockaddr_in>() as u32;
            } else {
                let ptr: *mut sockaddr_in6 = &mut send_sockaddrs_ipv6[i];

                msghdr.msg_name = ptr as *mut c_void;
                msghdr.msg_namelen = size_of::<sockaddr_in6>() as u32;
            }

            msghdr
        })
        .collect();
    
    // Setup ring and loop

    let timeout = Timespec::new().nsec(500_000_000);
    let mut timeout_set = false;

    let mut recv_entries = Slab::with_capacity(MAX_RECV_ENTRIES);
    let mut send_entries = Slab::with_capacity(MAX_SEND_ENTRIES);

    let mut ring = io_uring::IoUring::new(RING_SIZE as u32).unwrap();

    let (submitter, mut sq, mut cq) = ring.split();

    submitter.register_files(&[socket.as_raw_fd()]).unwrap();

    let fd = Fixed(0);

    let cleaning_duration = Duration::from_secs(config.cleaning.connection_cleaning_interval);

    let mut iter_counter = 0usize;
    let mut last_cleaning = Instant::now();

    loop {
        while let Some(entry) = cq.next() {
            let user_data: UserData = entry.user_data().into();

            match user_data {
                UserData::RecvMsg { slab_key } => {
                    recv_entries.remove(slab_key);

                    let result = entry.result();

                    if result < 0 {
                        ::log::info!(
                            "recvmsg error {}: {:#}",
                            result,
                            ::std::io::Error::from_raw_os_error(-result)
                        );
                    } else if result == 0 {
                        ::log::info!("recvmsg error: 0 bytes read");
                    } else {
                        let buffer_index = user_data.get_buffer_index();
                        let buffer_len = result as usize;

                        let addr = if config.network.address.is_ipv4() {
                            SocketAddr::V4(SocketAddrV4::new(
                                Ipv4Addr::from(u32::from_be(
                                    recv_sockaddrs_ipv4[buffer_index].sin_addr.s_addr,
                                )),
                                u16::from_be(recv_sockaddrs_ipv4[buffer_index].sin_port),
                            ))
                        } else {
                            let mut octets = recv_sockaddrs_ipv6[buffer_index].sin6_addr.s6_addr;
                            let port = u16::from_be(recv_sockaddrs_ipv6[buffer_index].sin6_port);

                            for byte in octets.iter_mut() {
                                *byte = u8::from_be(*byte);
                            }

                            let ip = match octets {
                                // Convert IPv4-mapped address (available in std but nightly-only)
                                [0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0xff, 0xff, a, b, c, d] => {
                                    Ipv4Addr::new(a, b, c, d).into()
                                }
                                octets => Ipv6Addr::from(octets).into(),
                            };

                            SocketAddr::new(ip, port)
                        };

                        let res_request = Request::from_bytes(
                            &recv_buffers[buffer_index][..buffer_len],
                            config.protocol.max_scrape_torrents,
                        );

                        // FIXME: don't run every iteration
                        let valid_until = ValidUntil::new(config.cleaning.max_connection_age);

                        handle_request(
                            &config,
                            &mut connections,
                            &mut access_list_cache,
                            &mut rng,
                            &request_sender,
                            &mut local_responses,
                            valid_until,
                            res_request,
                            addr,
                        );
                    }
                }
                UserData::SendMsg { slab_key } => {
                    send_entries.remove(slab_key);

                    if entry.result() < 0 {
                        ::log::error!(
                            "sendmsg error: {:#}",
                            ::std::io::Error::from_raw_os_error(-entry.result())
                        );
                    }
                }
                UserData::Timeout => {
                    timeout_set = false;
                }
            }
        }

        for _ in 0..(MAX_RECV_ENTRIES - recv_entries.len()) {
            let slab_key = recv_entries.insert(());
            let user_data = UserData::RecvMsg { slab_key };

            let msghdr_ptr: *mut msghdr = &mut recv_msghdrs[user_data.get_buffer_index()];

            let entry = io_uring::opcode::RecvMsg::new(fd, msghdr_ptr)
                .build()
                .user_data(user_data.into());

            unsafe {
                sq.push(&entry).unwrap();
            }
        }

        if !timeout_set {
            // Setup timer to occasionally check if there are pending responses
            let user_data = UserData::Timeout;

            let timespec_ptr: *const Timespec = &timeout;

            let entry = io_uring::opcode::Timeout::new(timespec_ptr)
                .build()
                .user_data(user_data.into());

            unsafe {
                sq.push(&entry).unwrap();
            }

            timeout_set = true;
        }

        let num_local_to_queue = (MAX_SEND_ENTRIES - send_entries.len()).min(local_responses.len());

        for (response, addr) in local_responses.drain(local_responses.len() - num_local_to_queue..)
        {
            queue_response(
                &config,
                &mut sq,
                fd,
                &mut send_entries,
                &mut send_buffers_connect,
                &mut send_buffers_announce_ipv4,
                &mut send_buffers_announce_ipv6,
                &mut send_buffers_scrape,
                &mut send_buffers_error,
                &mut send_iovs,
                &mut send_sockaddrs_ipv4,
                &mut send_sockaddrs_ipv6,
                &mut send_msghdrs,
                response,
                addr,
            );
        }

        for (response, addr) in response_receiver
            .try_iter()
            .take(MAX_SEND_ENTRIES - send_entries.len())
        {
            queue_response(
                &config,
                &mut sq,
                fd,
                &mut send_entries,
                &mut send_buffers_connect,
                &mut send_buffers_announce_ipv4,
                &mut send_buffers_announce_ipv6,
                &mut send_buffers_scrape,
                &mut send_buffers_error,
                &mut send_iovs,
                &mut send_sockaddrs_ipv4,
                &mut send_sockaddrs_ipv6,
                &mut send_msghdrs,
                response.into(),
                addr,
            );
        }

        if iter_counter % 32 == 0 {
            let now = Instant::now();

            if now > last_cleaning + cleaning_duration {
                connections.clean();

                last_cleaning = now;
            }
        }

        let all_responses_sent = local_responses.is_empty() & response_receiver.is_empty();

        let wait_for_num = if all_responses_sent {
            send_entries.len() + recv_entries.len()
        } else {
            send_entries.len()
        };

        sq.sync();

        submitter.submit_and_wait(wait_for_num).unwrap();

        sq.sync();
        cq.sync();

        iter_counter = iter_counter.wrapping_add(1);
    }
}

fn queue_response(
    config: &Config,
    sq: &mut SubmissionQueue,
    fd: Fixed,
    send_entries: &mut Slab<()>,
    buffers_connect: &mut [ConnectResponse],
    buffers_announce_ipv4: &mut [AnnounceResponseIpv4],
    buffers_announce_ipv6: &mut [AnnounceResponseIpv6],
    buffers_scrape: &mut [ScrapeResponse],
    buffers_error: &mut [ErrorResponse],
    iovs: &mut [[iovec; 2]],
    sockaddrs_ipv4: &mut [sockaddr_in],
    sockaddrs_ipv6: &mut [sockaddr_in6],
    msghdrs: &mut [msghdr],
    response: Response,
    addr: SocketAddr,
) {
    let slab_key = send_entries.insert(());
    let user_data = UserData::SendMsg { slab_key };

    let buffer_index = user_data.get_buffer_index();

    match response {
        Response::Connect(r) => {
            let buf = &mut buffers_connect[buffer_index];

            *buf = r;

            let buf: *mut ConnectResponse = buf;
            let len = size_of::<ConnectResponse>();

            iovs[buffer_index][0].iov_base = buf as *mut c_void;
            iovs[buffer_index][0].iov_len = len;

            iovs[buffer_index][1].iov_base = null_mut();
            iovs[buffer_index][1].iov_len = 0;
        }
        Response::AnnounceIpv4(r) => {
            let buf = &mut buffers_announce_ipv4[buffer_index];

            *buf = r;

            {
                let ptr: *mut AnnounceResponseIpv4Fixed = &mut buf.fixed;
                let len = size_of::<AnnounceResponseIpv4Fixed>();

                iovs[buffer_index][0].iov_base = ptr as *mut c_void;
                iovs[buffer_index][0].iov_len = len;
            }
            {
                let ptr: *mut ResponsePeerIpv4 = buf.peers.as_mut_ptr();
                let len = size_of::<ResponsePeerIpv4>() * buf.peers.len();

                iovs[buffer_index][1].iov_base = ptr as *mut c_void;
                iovs[buffer_index][1].iov_len = len;
            }
        }
        Response::AnnounceIpv6(r) => {
            let buf = &mut buffers_announce_ipv6[buffer_index];

            *buf = r;

            {
                let ptr: *mut AnnounceResponseIpv6Fixed = &mut buf.fixed;
                let len = size_of::<AnnounceResponseIpv6Fixed>();

                iovs[buffer_index][0].iov_base = ptr as *mut c_void;
                iovs[buffer_index][0].iov_len = len;
            }
            {
                let ptr: *mut ResponsePeerIpv6 = buf.peers.as_mut_ptr();
                let len = size_of::<ResponsePeerIpv6>() * buf.peers.len();

                iovs[buffer_index][1].iov_base = ptr as *mut c_void;
                iovs[buffer_index][1].iov_len = len;
            }
        }
        Response::Scrape(r) => {
            let buf = &mut buffers_scrape[buffer_index];

            *buf = r;

            {
                let ptr: *mut ScrapeResponseFixed = &mut buf.fixed;
                let len = size_of::<ScrapeResponseFixed>();

                iovs[buffer_index][0].iov_base = ptr as *mut c_void;
                iovs[buffer_index][0].iov_len = len;
            }
            {
                let ptr: *mut TorrentScrapeStatistics = buf.torrent_stats.as_mut_ptr();
                let len = size_of::<TorrentScrapeStatistics>() * buf.torrent_stats.len();

                iovs[buffer_index][1].iov_base = ptr as *mut c_void;
                iovs[buffer_index][1].iov_len = len;
            }
        }
        Response::Error(r) => {
            let buf = &mut buffers_error[buffer_index];

            *buf = r;

            {
                let ptr: *mut ErrorResponseFixed = &mut buf.fixed;
                let len = size_of::<ErrorResponseFixed>();

                iovs[buffer_index][0].iov_base = ptr as *mut c_void;
                iovs[buffer_index][0].iov_len = len;
            }
            {
                let ptr: *mut u8 = buf.message.as_mut_ptr();
                let len = buf.message.len();

                iovs[buffer_index][1].iov_base = ptr as *mut c_void;
                iovs[buffer_index][1].iov_len = len;
            }
        }
    }

    if config.network.address.is_ipv4() {
        let addr = if let SocketAddr::V4(addr) = addr {
            addr
        } else {
            unreachable!();
        };

        sockaddrs_ipv4[buffer_index].sin_addr.s_addr = u32::to_be((*addr.ip()).into());
        sockaddrs_ipv4[buffer_index].sin_port = u16::to_be(addr.port());
    } else {
        let mut octets = match addr {
            SocketAddr::V4(addr) => addr.ip().to_ipv6_mapped().octets(),
            SocketAddr::V6(addr) => addr.ip().octets(),
        };

        for byte in octets.iter_mut() {
            *byte = byte.to_be();
        }

        sockaddrs_ipv6[buffer_index].sin6_addr.s6_addr = octets;
        sockaddrs_ipv6[buffer_index].sin6_port = u16::to_be(addr.port());
    }

    let msghdr_ptr: *mut msghdr = &mut msghdrs[buffer_index];

    let entry = io_uring::opcode::SendMsg::new(fd, msghdr_ptr)
        .build()
        .user_data(user_data.into());

    unsafe {
        sq.push(&entry).unwrap();
    }
}

#[cfg(test)]
mod tests {
    use quickcheck::Arbitrary;
    use quickcheck_macros::quickcheck;

    use super::*;

    impl quickcheck::Arbitrary for UserData {
        fn arbitrary(g: &mut quickcheck::Gen) -> Self {
            match (bool::arbitrary(g), bool::arbitrary(g)) {
                (false, b) => {
                    let slab_key: u32 = Arbitrary::arbitrary(g);
                    let slab_key = slab_key as usize;

                    if b {
                        UserData::RecvMsg { slab_key }
                    } else {
                        UserData::SendMsg { slab_key }
                    }
                }
                _ => UserData::Timeout,
            }
        }
    }

    #[quickcheck]
    fn test_user_data_identity(a: UserData) -> bool {
        let n: u64 = a.into();
        let b = UserData::from(n);

        a == b
    }
}
