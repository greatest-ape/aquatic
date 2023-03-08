use std::{cell::UnsafeCell, io::Cursor, net::SocketAddr, ops::IndexMut, ptr::null_mut};

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

struct SendBuffer {
    name_v4: UnsafeCell<libc::sockaddr_in>,
    name_v6: UnsafeCell<libc::sockaddr_in6>,
    bytes: UnsafeCell<[u8; BUF_LEN]>,
    iovec: UnsafeCell<libc::iovec>,
    msghdr: UnsafeCell<libc::msghdr>,
    free: bool,
    // Only used for statistics
    receiver_is_ipv4: bool,
    // Only used for statistics
    response_type: ResponseType,
}

impl SendBuffer {
    fn new_with_null_pointers() -> Self {
        Self {
            name_v4: UnsafeCell::new(libc::sockaddr_in {
                sin_family: libc::AF_INET as u16,
                sin_port: 0,
                sin_addr: libc::in_addr { s_addr: 0 },
                sin_zero: [0; 8],
            }),
            name_v6: UnsafeCell::new(libc::sockaddr_in6 {
                sin6_family: libc::AF_INET6 as u16,
                sin6_port: 0,
                sin6_flowinfo: 0,
                sin6_addr: libc::in6_addr { s6_addr: [0; 16] },
                sin6_scope_id: 0,
            }),
            bytes: UnsafeCell::new([0; BUF_LEN]),
            iovec: UnsafeCell::new(libc::iovec {
                iov_base: null_mut(),
                iov_len: 0,
            }),
            msghdr: UnsafeCell::new(libc::msghdr {
                msg_name: null_mut(),
                msg_namelen: 0,
                msg_iov: null_mut(),
                msg_iovlen: 1,
                msg_control: null_mut(),
                msg_controllen: 0,
                msg_flags: 0,
            }),
            free: true,
            receiver_is_ipv4: true,
            response_type: ResponseType::Connect,
        }
    }

    /// # Safety
    ///
    /// - SendBuffer must be stored at a fixed location in memory
    unsafe fn setup_pointers(&mut self, socket_is_ipv4: bool) {
        let iovec = &mut *self.iovec.get();

        iovec.iov_base = self.bytes.get() as *mut libc::c_void;
        iovec.iov_len = (&*self.bytes.get()).len();

        let msghdr = &mut *self.msghdr.get();

        msghdr.msg_iov = self.iovec.get();

        if socket_is_ipv4 {
            msghdr.msg_name = self.name_v4.get() as *mut libc::c_void;
            msghdr.msg_namelen = core::mem::size_of::<libc::sockaddr_in>() as u32;
        } else {
            msghdr.msg_name = self.name_v6.get() as *mut libc::c_void;
            msghdr.msg_namelen = core::mem::size_of::<libc::sockaddr_in6>() as u32;
        }
    }

    /// # Safety
    ///
    /// - SendBuffer must be stored at a fixed location in memory
    /// - SendBuffer.setup_pointers must have been called previously
    unsafe fn prepare_entry(
        &mut self,
        response: &Response,
        addr: CanonicalSocketAddr,
        socket_is_ipv4: bool,
    ) -> Result<io_uring::squeue::Entry, Error> {
        // Set receiver socket addr
        if socket_is_ipv4 {
            self.receiver_is_ipv4 = true;

            let addr = if let Some(SocketAddr::V4(addr)) = addr.get_ipv4() {
                addr
            } else {
                panic!("ipv6 address in ipv4 mode");
            };

            let name = &mut *self.name_v4.get();

            name.sin_port = addr.port().to_be();
            name.sin_addr.s_addr = u32::from(*addr.ip()).to_be();
        } else {
            self.receiver_is_ipv4 = addr.is_ipv4();

            let addr = if let SocketAddr::V6(addr) = addr.get_ipv6_mapped() {
                addr
            } else {
                panic!("ipv4 address when ipv6 or ipv6-mapped address expected");
            };

            let name = &mut *self.name_v6.get();

            name.sin6_port = addr.port().to_be();
            name.sin6_addr.s6_addr = addr.ip().octets();
        }

        let bytes = (&mut *self.bytes.get()).as_mut_slice();

        let mut cursor = Cursor::new(bytes);

        match response.write(&mut cursor) {
            Ok(()) => {
                (&mut *self.iovec.get()).iov_len = cursor.position() as usize;

                self.response_type = ResponseType::from_response(response);
                self.free = false;

                let sqe = SendMsg::new(SOCKET_IDENTIFIER, self.msghdr.get()).build();

                Ok(sqe)
            }
            Err(err) => Err(Error::SerializationFailed(err)),
        }
    }
}

pub struct SendBuffers {
    likely_next_free_index: usize,
    socket_is_ipv4: bool,
    buffers: Box<[SendBuffer]>,
}

impl SendBuffers {
    pub fn new(config: &Config, capacity: usize) -> Self {
        let socket_is_ipv4 = config.network.address.is_ipv4();

        let mut buffers = ::std::iter::repeat_with(|| SendBuffer::new_with_null_pointers())
            .take(capacity)
            .collect::<Vec<_>>()
            .into_boxed_slice();

        for buffer in buffers.iter_mut() {
            // Safety: OK because buffers are stored in fixed memory location
            unsafe {
                buffer.setup_pointers(socket_is_ipv4);
            }
        }

        Self {
            likely_next_free_index: 0,
            socket_is_ipv4,
            buffers,
        }
    }

    pub fn response_type_and_ipv4(&self, index: usize) -> (ResponseType, bool) {
        let buffer = self.buffers.get(index).unwrap();

        (buffer.response_type, buffer.receiver_is_ipv4)
    }

    pub fn mark_index_as_free(&mut self, index: usize) {
        self.buffers[index].free = true;
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

        let buffer = self.buffers.index_mut(index);

        // Safety: OK because buffers are stored in fixed memory location
        // and buffer pointers were set up in SendBuffers::new()
        unsafe {
            match buffer.prepare_entry(response, addr, self.socket_is_ipv4) {
                Ok(entry) => {
                    self.likely_next_free_index = index + 1;

                    Ok(entry.user_data(index as u64))
                }
                Err(err) => Err(err),
            }
        }
    }

    fn next_free_index(&self) -> Result<usize, Error> {
        if self.likely_next_free_index >= self.buffers.len() {
            return Err(Error::NoBuffers);
        }

        for (i, buffer) in self.buffers[self.likely_next_free_index..]
            .iter()
            .enumerate()
        {
            if buffer.free {
                return Ok(self.likely_next_free_index + i);
            }
        }

        Err(Error::NoBuffers)
    }
}
