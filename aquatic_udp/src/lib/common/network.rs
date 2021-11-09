use nix::sys::socket::{sendmmsg, ControlMessage, InetAddr, MsgFlags, SendMmsgData};
use nix::sys::uio::IoVec;
use std::marker::PhantomData;
use std::net::IpAddr;
use std::{io::Cursor, net::SocketAddr, os::unix::prelude::AsRawFd, time::Instant};

use aquatic_common::AHashIndexMap;
pub use aquatic_common::{access_list::AccessList, ValidUntil};
pub use aquatic_udp_protocol::*;

use super::MAX_PACKET_SIZE;

const MAX_RESPONSES_PER_SYSCALL: usize = 32;

#[derive(Default)]
pub struct ConnectionMap(AHashIndexMap<(ConnectionId, SocketAddr), ValidUntil>);

impl ConnectionMap {
    pub fn insert(
        &mut self,
        connection_id: ConnectionId,
        socket_addr: SocketAddr,
        valid_until: ValidUntil,
    ) {
        self.0.insert((connection_id, socket_addr), valid_until);
    }

    pub fn contains(&self, connection_id: ConnectionId, socket_addr: SocketAddr) -> bool {
        self.0.contains_key(&(connection_id, socket_addr))
    }

    pub fn clean(&mut self) {
        let now = Instant::now();

        self.0.retain(|_, v| v.0 > now);
        self.0.shrink_to_fit();
    }
}

#[derive(Clone, Copy)]
struct PreparedResponse {
    buffer: [u8; MAX_PACKET_SIZE],
    len: usize,
    addr: nix::sys::socket::SockAddr,
}

pub struct ResponseSender {
    responses: [PreparedResponse; MAX_RESPONSES_PER_SYSCALL],
    num_to_send: usize,
}

impl Default for ResponseSender {
    fn default() -> Self {
        let empty = PreparedResponse {
            buffer: [0u8; MAX_PACKET_SIZE],
            len: 0,
            addr: convert_socket_addr(&SocketAddr::V4(::std::net::SocketAddrV4::new(::std::net::Ipv4Addr::UNSPECIFIED, 0))),
        };

        Self {
            responses: [empty; MAX_RESPONSES_PER_SYSCALL],
            num_to_send: 0,
        }
    }
}

impl ResponseSender {
    pub fn queue_and_maybe_send_response<S: AsRawFd>(
        &mut self,
        socket: &S,
        response: Response,
        addr: SocketAddr,
    ) {
        let prepared_response = &mut self.responses[self.num_to_send];

        let mut cursor = Cursor::new(&mut prepared_response.buffer[..]);

        response
            .write(&mut cursor, ip_version_from_ip(addr.ip()))
            .expect("write response");
        
        prepared_response.len = cursor.position() as usize;
        prepared_response.addr = convert_socket_addr(&addr);

        self.num_to_send += 1;

        if self.num_to_send == MAX_RESPONSES_PER_SYSCALL {
            self.flush(socket);
        }
    }

    pub fn flush<S: AsRawFd>(&mut self, socket: &S) {
        if self.num_to_send == 0 {
            return;
        }

        let control_messages: [ControlMessage; 0] = [];

        let mut io_vectors = Vec::with_capacity(self.num_to_send);

        for i in 0..self.num_to_send {
            let iov = [IoVec::from_slice(
                &self.responses[i].buffer[..self.responses[i].len],
            )];

            io_vectors.push(iov);
        }

        let mut messages = Vec::with_capacity(self.num_to_send);

        for i in 0..self.num_to_send {
            let message = SendMmsgData {
                iov: &io_vectors[i],
                cmsgs: &control_messages[..],
                addr: Some(self.responses[i].addr),
                _lt: PhantomData::default(),
            };

            messages.push(message);
        }

        match sendmmsg(socket.as_raw_fd(), &messages, MsgFlags::MSG_DONTWAIT) {
            Ok(_) => {}
            Err(err) => {
                ::log::error!("sendmmsg error: {:#}", err)
            }
        }

        self.num_to_send = 0;
    }
}

fn convert_socket_addr(addr: &SocketAddr) -> nix::sys::socket::SockAddr {
    nix::sys::socket::SockAddr::Inet(InetAddr::from_std(addr))
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
