use std::{io::Cursor, net::IpAddr, ptr::null_mut};

use aquatic_common::CanonicalSocketAddr;
use aquatic_udp_protocol::Response;
use io_uring::opcode::SendMsg;

use crate::config::Config;

use super::{BUF_LEN, SOCKET_FIXED};

pub struct SendBuffers {
    network_address: IpAddr,
    names_v4: Vec<libc::sockaddr_in>,
    names_v6: Vec<libc::sockaddr_in6>,
    buffers: Vec<[u8; BUF_LEN]>,
    iovecs: Vec<libc::iovec>,
    msghdrs: Vec<libc::msghdr>,
    free: Vec<bool>,
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
        response: &Response,
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
                    SendMsg::new(SOCKET_FIXED, msg_hdr)
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

    fn next_free_index(&mut self, index: usize) -> Option<usize> {
        for (i, free) in self.free[index..].iter().copied().enumerate() {
            if free {
                return Some(index + i);
            }
        }

        None
    }

    pub fn try_add(
        &mut self,
        index: usize,
        response: Response,
        addr: CanonicalSocketAddr,
    ) -> Result<(usize, io_uring::squeue::Entry), (Response, CanonicalSocketAddr)> {
        if let Some(index) = self.next_free_index(index) {
            if let Some(entry) = self.prepare_entry(index, &response, addr) {
                return Ok((index, entry));
            }
        }

        Err((response, addr))
    }

    pub fn mark_index_as_free(&mut self, index: usize) {
        self.free[index] = true;
    }
}
