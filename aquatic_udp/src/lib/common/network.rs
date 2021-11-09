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

pub struct ResponseSender {
    response_buffers: [[u8; MAX_PACKET_SIZE]; MAX_RESPONSES_PER_SYSCALL],
    response_lengths: [usize; MAX_RESPONSES_PER_SYSCALL],
    recepients: [Option<nix::sys::socket::SockAddr>; MAX_RESPONSES_PER_SYSCALL],
    response_index: usize,
}

impl Default for ResponseSender {
    fn default() -> Self {
        Self {
            response_buffers: [[0u8; MAX_PACKET_SIZE]; MAX_RESPONSES_PER_SYSCALL],
            response_lengths: [0usize; MAX_RESPONSES_PER_SYSCALL],
            recepients: [None; MAX_RESPONSES_PER_SYSCALL],
            response_index: 0,
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
        let mut buf = Cursor::new(&mut self.response_buffers[self.response_index][..]);

        response
            .write(&mut buf, ip_version_from_ip(addr.ip()))
            .expect("write response");

        self.response_lengths[self.response_index] = buf.position() as usize;
        self.recepients[self.response_index] = Some(Self::convert_socket_addr(&addr));

        if self.response_index == MAX_RESPONSES_PER_SYSCALL - 1 {
            self.force_send(socket);
        } else {
            self.response_index += 1;
        }
    }

    // TODO: call with timer with user-configurable interval
    fn force_send<S: AsRawFd>(&mut self, socket: &S) {
        let control_messages: [ControlMessage; 0] = [];
        let num_to_send = self.response_index + 1;

        let mut io_vectors = Vec::with_capacity(num_to_send);

        for i in 0..num_to_send {
            let iov = [IoVec::from_slice(
                &self.response_buffers[i][..self.response_lengths[i]],
            )];

            io_vectors.push(iov);
        }

        let mut messages = Vec::with_capacity(num_to_send);

        for i in 0..num_to_send {
            let message = SendMmsgData {
                iov: &io_vectors[i],
                cmsgs: &control_messages[..],
                addr: self.recepients[i],
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

        self.response_index = 0;
    }

    fn convert_socket_addr(addr: &SocketAddr) -> nix::sys::socket::SockAddr {
        nix::sys::socket::SockAddr::Inet(InetAddr::from_std(addr))
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
