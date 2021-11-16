use std::io::Cursor;
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
use crossbeam_channel::Receiver;
use io_uring::types::{Fixed, Timespec};
use io_uring::SubmissionQueue;
use libc::{
    c_void, in6_addr, in_addr, iovec, msghdr, sockaddr_in, sockaddr_in6, AF_INET, AF_INET6,
};
use rand::prelude::{SeedableRng, StdRng};
use slab::Slab;

use aquatic_udp_protocol::{Request, Response};

use crate::common::network::ConnectionMap;
use crate::common::network::*;
use crate::common::*;
use crate::config::Config;

const RING_SIZE: usize = 128;
const MAX_RECV_EVENTS: usize = 1;
const MAX_SEND_EVENTS: usize = RING_SIZE - MAX_RECV_EVENTS - 1;
const NUM_BUFFERS: usize = MAX_RECV_EVENTS + MAX_SEND_EVENTS;

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
            Self::SendMsg { slab_key } => slab_key + MAX_RECV_EVENTS,
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
    request_sender: ConnectedRequestSender,
    response_receiver: Receiver<(ConnectedResponse, SocketAddr)>,
    num_bound_sockets: Arc<AtomicUsize>,
) {
    let mut rng = StdRng::from_entropy();

    let socket = create_socket(&config);

    num_bound_sockets.fetch_add(1, Ordering::SeqCst);

    let mut connections = ConnectionMap::default();
    let mut pending_scrape_responses = PendingScrapeResponseMap::default();
    let mut access_list_cache = create_access_list_cache(&state.access_list);
    let mut local_responses: Vec<(Response, SocketAddr)> = Vec::new();

    let mut buffers: Vec<[u8; MAX_PACKET_SIZE]> =
        (0..NUM_BUFFERS).map(|_| [0; MAX_PACKET_SIZE]).collect();

    let mut sockaddrs_ipv4 = [sockaddr_in {
        sin_addr: in_addr { s_addr: 0 },
        sin_port: 0,
        sin_family: AF_INET as u16,
        sin_zero: Default::default(),
    }; NUM_BUFFERS];

    let mut sockaddrs_ipv6 = [sockaddr_in6 {
        sin6_addr: in6_addr { s6_addr: [0; 16] },
        sin6_port: 0,
        sin6_family: AF_INET6 as u16,
        sin6_flowinfo: 0,
        sin6_scope_id: 0,
    }; NUM_BUFFERS];

    let mut iovs: Vec<iovec> = (0..NUM_BUFFERS)
        .map(|i| {
            let iov_base = buffers[i].as_mut_ptr() as *mut c_void;
            let iov_len = MAX_PACKET_SIZE;

            iovec { iov_base, iov_len }
        })
        .collect();

    let mut msghdrs: Vec<msghdr> = (0..NUM_BUFFERS)
        .map(|i| {
            let msg_iov: *mut iovec = &mut iovs[i];

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
                let ptr: *mut sockaddr_in = &mut sockaddrs_ipv4[i];

                msghdr.msg_name = ptr as *mut c_void;
                msghdr.msg_namelen = size_of::<sockaddr_in>() as u32;
            } else {
                let ptr: *mut sockaddr_in6 = &mut sockaddrs_ipv6[i];

                msghdr.msg_name = ptr as *mut c_void;
                msghdr.msg_namelen = size_of::<sockaddr_in6>() as u32;
            }

            msghdr
        })
        .collect();

    let timeout = Timespec::new().nsec(100_000_000);

    let mut force_send_responses = false;
    let mut timeout_queued = false;

    let mut recv_entries = Slab::with_capacity(MAX_RECV_EVENTS);
    let mut send_entries = Slab::with_capacity(MAX_SEND_EVENTS);

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

                        let src = if config.network.address.is_ipv4() {
                            SocketAddr::V4(SocketAddrV4::new(
                                Ipv4Addr::from(u32::from_be(
                                    sockaddrs_ipv4[buffer_index].sin_addr.s_addr,
                                )),
                                u16::from_be(sockaddrs_ipv4[buffer_index].sin_port),
                            ))
                        } else {
                            let mut octets = sockaddrs_ipv6[buffer_index].sin6_addr.s6_addr;
                            let port = u16::from_be(sockaddrs_ipv6[buffer_index].sin6_port);

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
                            &buffers[buffer_index][..buffer_len],
                            config.protocol.max_scrape_torrents,
                        );

                        // FIXME: don't run every iteration
                        let valid_until = ValidUntil::new(config.cleaning.max_connection_age);

                        handle_request(
                            &config,
                            &mut connections,
                            &mut pending_scrape_responses,
                            &mut access_list_cache,
                            &mut rng,
                            &request_sender,
                            &mut local_responses,
                            valid_until,
                            res_request,
                            src,
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
                    force_send_responses = true;
                    timeout_queued = false;
                }
            }
        }

        for _ in 0..(MAX_RECV_EVENTS - recv_entries.len()) {
            let slab_key = recv_entries.insert(());
            let user_data = UserData::RecvMsg { slab_key };

            let msghdr_ptr: *mut msghdr = &mut msghdrs[user_data.get_buffer_index()];

            let entry = io_uring::opcode::RecvMsg::new(fd, msghdr_ptr)
                .build()
                .user_data(user_data.into());

            unsafe {
                sq.push(&entry).unwrap();
            }
        }

        for (response, addr) in response_receiver.try_iter() {
            let opt_response = match response {
                ConnectedResponse::Scrape(r) => pending_scrape_responses.add_and_get_finished(r),
                ConnectedResponse::AnnounceIpv4(r) => Some(Response::AnnounceIpv4(r)),
                ConnectedResponse::AnnounceIpv6(r) => Some(Response::AnnounceIpv6(r)),
            };

            if let Some(response) = opt_response {
                local_responses.push((response, addr));
            }
        }

        let space_in_send_queue = MAX_SEND_EVENTS - send_entries.len();

        if force_send_responses | (local_responses.len() >= space_in_send_queue) {
            let num_to_queue = (space_in_send_queue).min(local_responses.len());
            let drain_from_index = local_responses.len() - num_to_queue;

            for (response, addr) in local_responses.drain(drain_from_index..) {
                queue_response(
                    &config,
                    &mut sq,
                    fd,
                    &mut send_entries,
                    &mut buffers,
                    &mut iovs,
                    &mut sockaddrs_ipv4,
                    &mut sockaddrs_ipv6,
                    &mut msghdrs,
                    response,
                    addr,
                );
            }

            if local_responses.is_empty() {
                force_send_responses = false;
            }
        }

        if !timeout_queued & !force_send_responses {
            // Setup timer to occasionally force sending of responses
            let user_data = UserData::Timeout;

            let timespec_ptr: *const Timespec = &timeout;

            let entry = io_uring::opcode::Timeout::new(timespec_ptr)
                .build()
                .user_data(user_data.into());

            unsafe {
                sq.push(&entry).unwrap();
            }

            timeout_queued = true;
        }

        if iter_counter % 32 == 0 {
            let now = Instant::now();

            if now > last_cleaning + cleaning_duration {
                connections.clean();

                last_cleaning = now;
            }
        }

        let wait_for_num = if force_send_responses {
            send_entries.len()
        } else {
            send_entries.len() + recv_entries.len()
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
    buffers: &mut [[u8; MAX_PACKET_SIZE]],
    iovs: &mut [iovec],
    sockaddrs_ipv4: &mut [sockaddr_in],
    sockaddrs_ipv6: &mut [sockaddr_in6],
    msghdrs: &mut [msghdr],
    response: Response,
    addr: SocketAddr,
) {
    let slab_key = send_entries.insert(());
    let user_data = UserData::SendMsg { slab_key };

    let buffer_index = user_data.get_buffer_index();

    let mut cursor = Cursor::new(&mut buffers[buffer_index][..]);

    match response.write(&mut cursor) {
        Ok(()) => {
            iovs[buffer_index].iov_len = cursor.position() as usize;

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
        }
        Err(err) => {
            ::log::error!("Response::write error: {:?}", err);

            send_entries.remove(slab_key);

            return;
        }
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
