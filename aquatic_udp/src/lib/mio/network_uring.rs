use std::io::Cursor;
use std::mem::size_of_val;
use std::net::{IpAddr, Ipv4Addr, SocketAddr, SocketAddrV4};
use std::os::unix::prelude::{AsRawFd};
use std::ptr::{null_mut};
use std::sync::{
    atomic::{AtomicUsize, Ordering},
    Arc,
};
use std::time::{Duration, Instant};

use aquatic_common::access_list::{AccessListCache, create_access_list_cache};
use crossbeam_channel::{Receiver, Sender};
use io_uring::SubmissionQueue;
use io_uring::types::{Fixed, Timespec};
use libc::{c_void, in_addr, iovec, msghdr, sockaddr_in};
use rand::prelude::{Rng, SeedableRng, StdRng};
use slab::Slab;
use socket2::{Domain, Protocol, Socket, Type};

use aquatic_udp_protocol::{IpVersion, Request, Response};

use crate::common::handlers::*;
use crate::common::network::ConnectionMap;
use crate::common::*;
use crate::config::Config;

use super::common::*;

const RING_SIZE: usize = 128;
const MAX_RECV_EVENTS: usize = 1;
const MAX_SEND_EVENTS: usize = RING_SIZE - MAX_RECV_EVENTS - 1;
const NUM_BUFFERS: usize = MAX_RECV_EVENTS + MAX_SEND_EVENTS;

#[derive(Clone, Copy, Debug, PartialEq)]
enum Event {
    RecvMsg,
    SendMsg,
    Timeout,
}

#[derive(Clone, Copy, Debug, PartialEq)]
struct UserData {
    event: Event,
    slab_key: usize,
}

impl UserData {
    fn get_buffer_index(&self) -> usize {
        match self.event {
            Event::RecvMsg => {
                self.slab_key
            }
            Event::SendMsg => {
                self.slab_key + MAX_RECV_EVENTS
            }
            Event::Timeout => {
                unreachable!()
            }
        }
    }
}

impl From<u64> for UserData {
    fn from(mut n: u64) -> UserData {
        let bytes = bytemuck::bytes_of_mut(&mut n);

        let event = match bytes[7] {
            0 => Event::RecvMsg,
            1 => Event::SendMsg,
            2 => Event::Timeout,
            _ => unreachable!(),
        };

        bytes[7] = 0;

        UserData {
            event,
            slab_key: n as usize,
        }
    }
}

impl Into<u64> for UserData {
    fn into(self) -> u64 {
        let mut out = self.slab_key as u64;

        let bytes = bytemuck::bytes_of_mut(&mut out);

        bytes[7] = match self.event {
            Event::RecvMsg => 0,
            Event::SendMsg => 1,
            Event::Timeout => 2,
        };

        out
    }
}

pub fn run_socket_worker(
    state: State,
    config: Config,
    token_num: usize,
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

    let cleaning_duration = Duration::from_secs(config.cleaning.connection_cleaning_interval);

    let mut iter_counter = 0usize;
    let mut last_cleaning = Instant::now();

    let mut buffers: Vec<[u8; MAX_PACKET_SIZE]> = (0..NUM_BUFFERS).map(|_| [0; MAX_PACKET_SIZE]).collect();

    let mut sockaddrs_ipv4 = [
        sockaddr_in {
            sin_addr: in_addr {
                s_addr: 0,
            },
            sin_port: 0,
            sin_family: 0,
            sin_zero: Default::default(),
        }
        ; NUM_BUFFERS
    ];

    let mut iovs: Vec<iovec> = (0..NUM_BUFFERS).map(|i| {
        let iov_base = buffers[i].as_mut_ptr() as *mut c_void;
        let iov_len = MAX_PACKET_SIZE;

        iovec {
            iov_base,
            iov_len,
        }
    }).collect();

    let mut msghdrs: Vec<msghdr> = (0..NUM_BUFFERS).map(|i| {
        let msg_iov: *mut iovec = &mut iovs[i];
        let msg_name: *mut sockaddr_in = &mut sockaddrs_ipv4[i];

        msghdr {
            msg_name: msg_name as *mut c_void,
            msg_namelen: size_of_val(&sockaddrs_ipv4[i]) as u32,
            msg_iov,
            msg_iovlen: 1,
            msg_control: null_mut(),
            msg_controllen: 0,
            msg_flags: 0,
        }
    }).collect();

    let timeout = Timespec::new().nsec(500_000_000);
    let mut timeout_set = false;

    let mut recv_entries = Slab::with_capacity(MAX_RECV_EVENTS);
    let mut send_entries = Slab::with_capacity(MAX_SEND_EVENTS);

    let mut ring = io_uring::IoUring::new(RING_SIZE as u32).unwrap();

    let (submitter, mut sq, mut cq) = ring.split();

    submitter.register_files(&[socket.as_raw_fd()]).unwrap();

    let fd = Fixed(0);

    loop {
        while let Some(entry) = cq.next() {
            let user_data: UserData = entry.user_data().into();

            match user_data.event {
                Event::RecvMsg => {
                    recv_entries.remove(user_data.slab_key);

                    let result = entry.result();

                    if result < 0 {
                        ::log::info!("recvmsg error {}: {:#}", result, ::std::io::Error::from_raw_os_error(-result));
                    } else if result == 0 {
                        ::log::info!("recvmsg error: 0 bytes read");
                    } else {
                        let buffer_index = user_data.get_buffer_index();
                        let buffer_len = result as usize;

                        let src = SocketAddrV4::new(
                            Ipv4Addr::from(u32::from_be(sockaddrs_ipv4[buffer_index].sin_addr.s_addr)),
                            u16::from_be(sockaddrs_ipv4[buffer_index].sin_port),
                        );

                        let res_request =
                            Request::from_bytes(&buffers[buffer_index][..buffer_len], config.protocol.max_scrape_torrents);

                        handle_request(
                            &config,
                            &state,
                            &mut connections,
                            &mut access_list_cache,
                            &mut rng,
                            &request_sender,
                            &mut local_responses,
                            res_request,
                            SocketAddr::V4(src),
                        );
                    }
                }
                Event::SendMsg => {
                    send_entries.remove(user_data.slab_key);

                    if entry.result() < 0 {
                        ::log::info!("recvmsg error: {:#}", ::std::io::Error::from_raw_os_error(-entry.result()));
                    }
                }
                Event::Timeout => {
                    timeout_set = false;
                }
            }
        }

        for _ in 0..(MAX_RECV_EVENTS - recv_entries.len()) {
            let user_data = UserData {
                event: Event::RecvMsg,
                slab_key: recv_entries.insert(()),
            };

            let buffer_index = user_data.get_buffer_index();

            let buf_ptr: *mut msghdr = &mut msghdrs[buffer_index];

            let entry = io_uring::opcode::RecvMsg::new(fd, buf_ptr).build().user_data(user_data.into());

            unsafe {
                sq.push(&entry).unwrap();
            }
        }

        sq.sync();

        if !timeout_set {
            let user_data = UserData {
                event: Event::Timeout,
                slab_key: 0,
            };

            let timespec_ptr: *const Timespec = &timeout;

            let entry = io_uring::opcode::Timeout::new(timespec_ptr).build().user_data(user_data.into());

            unsafe {
                sq.push(&entry).unwrap();
            }

            timeout_set = true;
        }

        local_responses.extend(response_receiver
            .try_iter()
            .map(|(response, addr)| (response.into(), addr)));
        
        let num_responses_to_queue = (MAX_SEND_EVENTS - send_entries.len()).min(local_responses.len());
        
        for (response, src) in local_responses.drain(local_responses.len() - num_responses_to_queue..) {
            queue_response(&mut sq, fd, &mut send_entries, &mut buffers, &mut iovs, &mut sockaddrs_ipv4, &mut msghdrs, response, src);
        }

        sq.sync();

        if iter_counter % 32 == 0 {
            let now = Instant::now();

            if now > last_cleaning + cleaning_duration {
                connections.clean();

                last_cleaning = now;
            }
        }

        let wait_for_num = if local_responses.is_empty() {
            send_entries.len() + recv_entries.len()
        } else {
            send_entries.len()
        };

        submitter.submit_and_wait(wait_for_num).unwrap();

        sq.sync();
        cq.sync();

        iter_counter = iter_counter.wrapping_add(1);
    }
}

fn queue_response(
    sq: &mut SubmissionQueue,
    fd: Fixed,
    send_events: &mut Slab<()>,
    buffers: &mut [[u8; MAX_PACKET_SIZE]],
    iovs: &mut [iovec],
    sockaddrs: &mut [sockaddr_in],
    msghdrs: &mut [msghdr],
    response: Response,
    src: SocketAddr,
) {
    let user_data = UserData {
        event: Event::SendMsg,
        slab_key: send_events.insert(()),
    };

    let buffer_index = user_data.get_buffer_index();

    let mut cursor = Cursor::new(&mut buffers[buffer_index][..]);

    match response.write(&mut cursor, ip_version_from_ip(src.ip())) {
        Ok(()) => {
            iovs[buffer_index].iov_len = cursor.position() as usize;

            let src = if let SocketAddr::V4(src) = src {
                src
            } else {
                return; // FIXME
            };

            sockaddrs[buffer_index].sin_addr.s_addr = u32::to_be((*src.ip()).into());
            sockaddrs[buffer_index].sin_port = u16::to_be(src.port());
        }
        Err(err) => {
            ::log::error!("Response::write error: {:?}", err);
        }
    }

    let buf_ptr: *mut msghdr = &mut msghdrs[buffer_index];

    let entry = io_uring::opcode::SendMsg::new(fd, buf_ptr).build().user_data(user_data.into());

    unsafe {
        sq.push(&entry).unwrap();
    }
}

fn create_socket(config: &Config) -> ::std::net::UdpSocket {
    let socket = if config.network.address.is_ipv4() {
        Socket::new(Domain::IPV4, Type::DGRAM, Some(Protocol::UDP))
    } else {
        Socket::new(Domain::IPV6, Type::DGRAM, Some(Protocol::UDP))
    }
    .expect("create socket");

    socket.set_reuse_port(true).expect("socket: set reuse port");

    socket
        .set_nonblocking(true)
        .expect("socket: set nonblocking");

    socket
        .bind(&config.network.address.into())
        .unwrap_or_else(|err| panic!("socket: bind to {}: {:?}", config.network.address, err));

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

    socket.into()
}

#[inline]
fn handle_request(
    config: &Config,
    state: &State,
    connections: &mut ConnectionMap,
    access_list_cache: &mut AccessListCache,
    rng: &mut StdRng,
    request_sender: &Sender<(ConnectedRequest, SocketAddr)>,
    local_responses: &mut Vec<(Response, SocketAddr)>,
    res_request: Result<Request, RequestParseError>,
    src: SocketAddr,
) {

    let valid_until = ValidUntil::new(config.cleaning.max_connection_age);
    let access_list_mode = config.access_list.mode;

    match res_request {
        Ok(Request::Connect(request)) => {
            let connection_id = ConnectionId(rng.gen());

            connections.insert(connection_id, src, valid_until);

            let response = Response::Connect(ConnectResponse {
                connection_id,
                transaction_id: request.transaction_id,
            });

            local_responses.push((response, src))
        }
        Ok(Request::Announce(request)) => {
            if connections.contains(request.connection_id, src) {
                if access_list_cache
                    .load()
                    .allows(access_list_mode, &request.info_hash.0)
                {
                    if let Err(err) = request_sender
                        .try_send((ConnectedRequest::Announce(request), src))
                    {
                        ::log::warn!("request_sender.try_send failed: {:?}", err)
                    }
                } else {
                    let response = Response::Error(ErrorResponse {
                        transaction_id: request.transaction_id,
                        message: "Info hash not allowed".into(),
                    });

                    local_responses.push((response, src))
                }
            }
        }
        Ok(Request::Scrape(request)) => {
            if connections.contains(request.connection_id, src) {
                let request = ConnectedRequest::Scrape {
                    request,
                    original_indices: Vec::new(),
                };

                if let Err(err) = request_sender.try_send((request, src)) {
                    ::log::warn!("request_sender.try_send failed: {:?}", err)
                }
            }
        }
        Err(err) => {
            ::log::debug!("Request::from_bytes error: {:?}", err);

            if let RequestParseError::Sendable {
                connection_id,
                transaction_id,
                err,
            } = err
            {
                if connections.contains(connection_id, src) {
                    let response = ErrorResponse {
                        transaction_id,
                        message: err.right_or("Parse error").into(),
                    };

                    local_responses.push((response.into(), src));
                }
            }
        }
    }

}

fn ip_version_from_ip(ip: IpAddr) -> IpVersion {
    match ip {
        IpAddr::V4(_) => IpVersion::IPv4,
        IpAddr::V6(ip) => {
            if let [0, 0, 0, 0, 0, 0xffff, ..] = ip.segments() {
                IpVersion::IPv4
            } else {
                IpVersion::IPv6
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use quickcheck::Arbitrary;
    use quickcheck_macros::quickcheck;

    use super::*;

    impl quickcheck::Arbitrary for Event {
        fn arbitrary(g: &mut quickcheck::Gen) -> Self {
            if bool::arbitrary(g) {
                Event::RecvMsg
            } else {
                Event::SendMsg
            }
        }
    }

    impl quickcheck::Arbitrary for UserData {
        fn arbitrary(g: &mut quickcheck::Gen) -> Self {
            let slab_key: u32 = Arbitrary::arbitrary(g);

            Self {
                event: Arbitrary::arbitrary(g),
                slab_key: slab_key as usize,
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