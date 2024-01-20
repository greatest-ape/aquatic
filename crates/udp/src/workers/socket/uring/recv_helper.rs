use std::{
    net::{Ipv6Addr, SocketAddr, SocketAddrV4, SocketAddrV6},
    ptr::null_mut,
};

use aquatic_common::CanonicalSocketAddr;
use aquatic_udp_protocol::{Request, RequestParseError};
use io_uring::{opcode::RecvMsgMulti, types::RecvMsgOut};

use crate::config::Config;

use super::{SOCKET_IDENTIFIER, USER_DATA_RECV};

#[allow(clippy::enum_variant_names)]
pub enum Error {
    RecvMsgParseError,
    RecvMsgTruncated,
    RequestParseError(RequestParseError, CanonicalSocketAddr),
    InvalidSocketAddress,
}

pub struct RecvHelper {
    socket_is_ipv4: bool,
    max_scrape_torrents: u8,
    #[allow(dead_code)]
    name_v4: *const libc::sockaddr_in,
    msghdr_v4: *const libc::msghdr,
    #[allow(dead_code)]
    name_v6: *const libc::sockaddr_in6,
    msghdr_v6: *const libc::msghdr,
}

impl RecvHelper {
    pub fn new(config: &Config) -> Self {
        let name_v4 = Box::into_raw(Box::new(libc::sockaddr_in {
            sin_family: 0,
            sin_port: 0,
            sin_addr: libc::in_addr { s_addr: 0 },
            sin_zero: [0; 8],
        }));

        let msghdr_v4 = Box::into_raw(Box::new(libc::msghdr {
            msg_name: name_v4 as *mut libc::c_void,
            msg_namelen: core::mem::size_of::<libc::sockaddr_in>() as u32,
            msg_iov: null_mut(),
            msg_iovlen: 0,
            msg_control: null_mut(),
            msg_controllen: 0,
            msg_flags: 0,
        }));

        let name_v6 = Box::into_raw(Box::new(libc::sockaddr_in6 {
            sin6_family: 0,
            sin6_port: 0,
            sin6_flowinfo: 0,
            sin6_addr: libc::in6_addr { s6_addr: [0; 16] },
            sin6_scope_id: 0,
        }));

        let msghdr_v6 = Box::into_raw(Box::new(libc::msghdr {
            msg_name: name_v6 as *mut libc::c_void,
            msg_namelen: core::mem::size_of::<libc::sockaddr_in6>() as u32,
            msg_iov: null_mut(),
            msg_iovlen: 0,
            msg_control: null_mut(),
            msg_controllen: 0,
            msg_flags: 0,
        }));

        Self {
            socket_is_ipv4: config.network.address.is_ipv4(),
            max_scrape_torrents: config.protocol.max_scrape_torrents,
            name_v4,
            msghdr_v4,
            name_v6,
            msghdr_v6,
        }
    }

    pub fn create_entry(&self, buf_group: u16) -> io_uring::squeue::Entry {
        let msghdr = if self.socket_is_ipv4 {
            self.msghdr_v4
        } else {
            self.msghdr_v6
        };

        RecvMsgMulti::new(SOCKET_IDENTIFIER, msghdr, buf_group)
            .build()
            .user_data(USER_DATA_RECV)
    }

    pub fn parse(&self, buffer: &[u8]) -> Result<(Request, CanonicalSocketAddr), Error> {
        let (msg, addr) = if self.socket_is_ipv4 {
            // Safe as long as kernel only reads from the pointer and doesn't
            // write to it. I think this is the case.
            let msghdr = unsafe { self.msghdr_v4.read() };

            let msg = RecvMsgOut::parse(buffer, &msghdr).map_err(|_| Error::RecvMsgParseError)?;

            if msg.is_name_data_truncated() | msg.is_payload_truncated() {
                return Err(Error::RecvMsgTruncated);
            }

            let name_data = unsafe { *(msg.name_data().as_ptr() as *const libc::sockaddr_in) };

            let addr = SocketAddr::V4(SocketAddrV4::new(
                u32::from_be(name_data.sin_addr.s_addr).into(),
                u16::from_be(name_data.sin_port),
            ));

            (msg, addr)
        } else {
            // Safe as long as kernel only reads from the pointer and doesn't
            // write to it. I think this is the case.
            let msghdr = unsafe { self.msghdr_v6.read() };

            let msg = RecvMsgOut::parse(buffer, &msghdr).map_err(|_| Error::RecvMsgParseError)?;

            if msg.is_name_data_truncated() | msg.is_payload_truncated() {
                return Err(Error::RecvMsgTruncated);
            }

            let name_data = unsafe { *(msg.name_data().as_ptr() as *const libc::sockaddr_in6) };

            let addr = SocketAddr::V6(SocketAddrV6::new(
                Ipv6Addr::from(name_data.sin6_addr.s6_addr),
                u16::from_be(name_data.sin6_port),
                u32::from_be(name_data.sin6_flowinfo),
                u32::from_be(name_data.sin6_scope_id),
            ));

            (msg, addr)
        };

        if addr.port() == 0 {
            return Err(Error::InvalidSocketAddress);
        }

        let addr = CanonicalSocketAddr::new(addr);

        let request = Request::from_bytes(msg.payload_data(), self.max_scrape_torrents)
            .map_err(|err| Error::RequestParseError(err, addr))?;

        Ok((request, addr))
    }
}
