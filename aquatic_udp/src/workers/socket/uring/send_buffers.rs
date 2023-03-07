use std::{io::Cursor, net::IpAddr, ops::IndexMut, ptr::null_mut};

use aquatic_common::CanonicalSocketAddr;
use aquatic_udp_protocol::Response;
use io_uring::opcode::SendMsg;

use crate::config::Config;

use super::{BUF_LEN, SOCKET_IDENTIFIER};

pub enum Error {
    NoBuffers,
    SerializationFailed(std::io::Error),
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ResponseType {
    Connect,
    Announce,
    Scrape,
    Error,
}

impl ResponseType {
    fn from_response(response: &Response) -> Self {
        match response {
            Response::Connect(_) => Self::Connect,
            Response::AnnounceIpv4(_) | Response::AnnounceIpv6(_) => Self::Announce,
            Response::Scrape(_) => Self::Scrape,
            Response::Error(_) => Self::Error,
        }
    }
}

pub struct SendBuffers {
    likely_next_free_index: usize,
    network_address: IpAddr,
    names_v4: Vec<libc::sockaddr_in>,
    names_v6: Vec<libc::sockaddr_in6>,
    buffers: Vec<[u8; BUF_LEN]>,
    iovecs: Vec<libc::iovec>,
    msghdrs: Vec<libc::msghdr>,
    free: Vec<bool>,
    // Only used for statistics
    receiver_is_ipv4: Vec<bool>,
    // Only used for statistics
    response_types: Vec<ResponseType>,
}

impl SendBuffers {
    pub fn new(config: &Config, capacity: usize) -> Self {
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
                sin_family: libc::AF_INET as u16,
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
                    msg_name: msg_name as *mut _ as *mut libc::c_void,
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
                sin6_family: libc::AF_INET6 as u16,
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
                    msg_name: msg_name as *mut _ as *mut libc::c_void,
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
            likely_next_free_index: 0,
            network_address: config.network.address.ip(),
            names_v4,
            names_v6,
            buffers,
            iovecs,
            msghdrs,
            free: ::std::iter::repeat(true).take(capacity).collect(),
            receiver_is_ipv4: ::std::iter::repeat(true).take(capacity).collect(),
            response_types: ::std::iter::repeat(ResponseType::Connect)
                .take(capacity)
                .collect(),
        }
    }

    pub fn receiver_is_ipv4(&mut self, index: usize) -> bool {
        self.receiver_is_ipv4[index]
    }

    pub fn response_type(&mut self, index: usize) -> ResponseType {
        self.response_types[index]
    }

    pub fn mark_index_as_free(&mut self, index: usize) {
        self.free[index] = true;
    }

    /// Call after going through completion queue
    pub fn reset_index(&mut self) {
        self.likely_next_free_index = 0;
    }

    pub fn prepare_entry(
        &mut self,
        response: &Response,
        addr: CanonicalSocketAddr,
    ) -> Result<io_uring::squeue::Entry, Error> {
        let index = self.next_free_index()?;

        // Set receiver socket addr
        if self.network_address.is_ipv4() {
            let msg_name = self.names_v4.index_mut(index);
            let addr = addr.get_ipv4().unwrap();

            msg_name.sin_port = addr.port().to_be();
            msg_name.sin_addr.s_addr = if let IpAddr::V4(addr) = addr.ip() {
                u32::from(addr).to_be()
            } else {
                panic!("ipv6 address in ipv4 mode");
            };

            self.receiver_is_ipv4[index] = true;
        } else {
            let msg_name = self.names_v6.index_mut(index);
            let addr = addr.get_ipv6_mapped();

            msg_name.sin6_port = addr.port().to_be();
            msg_name.sin6_addr.s6_addr = if let IpAddr::V6(addr) = addr.ip() {
                addr.octets()
            } else {
                panic!("ipv4 address when ipv6 or ipv6-mapped address expected");
            };

            self.receiver_is_ipv4[index] = addr.is_ipv4();
        }

        let mut cursor = Cursor::new(self.buffers.index_mut(index).as_mut_slice());

        match response.write(&mut cursor) {
            Ok(()) => {
                self.iovecs[index].iov_len = cursor.position() as usize;
                self.response_types[index] = ResponseType::from_response(response);
                self.free[index] = false;

                self.likely_next_free_index = index + 1;

                let sqe = SendMsg::new(SOCKET_IDENTIFIER, self.msghdrs.index_mut(index))
                    .build()
                    .user_data(index as u64);

                Ok(sqe)
            }
            Err(err) => Err(Error::SerializationFailed(err)),
        }
    }

    fn next_free_index(&self) -> Result<usize, Error> {
        if self.likely_next_free_index >= self.free.len() {
            return Err(Error::NoBuffers);
        }

        for (i, free) in self.free[self.likely_next_free_index..]
            .iter()
            .copied()
            .enumerate()
        {
            if free {
                return Ok(self.likely_next_free_index + i);
            }
        }

        Err(Error::NoBuffers)
    }
}
