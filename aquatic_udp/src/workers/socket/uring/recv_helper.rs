use std::{
    net::{IpAddr, Ipv6Addr, SocketAddr, SocketAddrV4, SocketAddrV6},
    ptr::null_mut,
};

use aquatic_common::CanonicalSocketAddr;
use aquatic_udp_protocol::{Request, RequestParseError};
use io_uring::{opcode::RecvMsgMulti, types::RecvMsgOut};

use crate::config::Config;

use super::{RECV_USER_DATA, SOCKET_FIXED};

pub struct RecvHelper {
    network_address: IpAddr,
    max_scrape_torrents: u8,
    #[allow(dead_code)]
    name_v4: Box<libc::sockaddr_in>,
    msghdr_v4: Box<libc::msghdr>,
    #[allow(dead_code)]
    name_v6: Box<libc::sockaddr_in6>,
    msghdr_v6: Box<libc::msghdr>,
}

impl RecvHelper {
    pub fn new(config: &Config) -> Self {
        let mut name_v4 = Box::new(libc::sockaddr_in {
            sin_family: 0,
            sin_port: 0,
            sin_addr: libc::in_addr { s_addr: 0 },
            sin_zero: [0; 8],
        });

        let msghdr_v4 = Box::new(libc::msghdr {
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

        let msghdr_v6 = Box::new(libc::msghdr {
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

        RecvMsgMulti::new(SOCKET_FIXED, msghdr, buf_group)
            .build()
            .user_data(RECV_USER_DATA)
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
