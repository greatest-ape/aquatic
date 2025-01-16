use std::{
    io::Cursor,
    iter::repeat_with,
    mem::MaybeUninit,
    net::SocketAddr,
    ptr::{addr_of_mut, null_mut},
};

use aquatic_common::CanonicalSocketAddr;
use aquatic_udp_protocol::Response;
use io_uring::opcode::SendMsg;

use super::{RESPONSE_BUF_LEN, SOCKET_IDENTIFIER_V4, SOCKET_IDENTIFIER_V6};

pub enum Error {
    NoBuffers(Response),
    SerializationFailed(std::io::Error),
}

pub struct SendBuffers {
    likely_next_free_index: usize,
    buffers: Vec<(SendBufferMetadata, *mut SendBuffer)>,
}

impl SendBuffers {
    pub fn new(capacity: usize) -> Self {
        let buffers = repeat_with(|| (Default::default(), SendBuffer::new()))
            .take(capacity)
            .collect::<Vec<_>>();

        Self {
            likely_next_free_index: 0,
            buffers,
        }
    }

    pub fn response_type_and_ipv4(&self, index: usize) -> (ResponseType, bool) {
        let meta = &self.buffers.get(index).unwrap().0;

        (meta.response_type, meta.receiver_is_ipv4)
    }

    /// # Safety
    ///
    /// Only safe to call once buffer is no longer referenced by in-flight
    /// io_uring queue entries
    pub unsafe fn mark_buffer_as_free(&mut self, index: usize) {
        self.buffers[index].0.free = true;
    }

    /// Call after going through completion queue
    pub fn reset_likely_next_free_index(&mut self) {
        self.likely_next_free_index = 0;
    }

    pub fn prepare_entry(
        &mut self,
        send_to_ipv4_socket: bool,
        response: Response,
        addr: CanonicalSocketAddr,
    ) -> Result<io_uring::squeue::Entry, Error> {
        let index = if let Some(index) = self.next_free_index() {
            index
        } else {
            return Err(Error::NoBuffers(response));
        };

        let (buffer_metadata, buffer) = self.buffers.get_mut(index).unwrap();

        // Safe as long as `mark_buffer_as_free` was used correctly
        let buffer = unsafe { &mut *(*buffer) };

        match buffer.prepare_entry(response, addr, send_to_ipv4_socket, buffer_metadata) {
            Ok(entry) => {
                buffer_metadata.free = false;

                self.likely_next_free_index = index + 1;

                Ok(entry.user_data(index as u64))
            }
            Err(err) => Err(err),
        }
    }

    fn next_free_index(&self) -> Option<usize> {
        if self.likely_next_free_index >= self.buffers.len() {
            return None;
        }

        for (i, (meta, _)) in self.buffers[self.likely_next_free_index..]
            .iter()
            .enumerate()
        {
            if meta.free {
                return Some(self.likely_next_free_index + i);
            }
        }

        None
    }
}

/// Make sure not to hold any reference to this struct while kernel can
/// write to its contents
struct SendBuffer {
    name_v4: libc::sockaddr_in,
    name_v6: libc::sockaddr_in6,
    bytes: [u8; RESPONSE_BUF_LEN],
    iovec: libc::iovec,
    msghdr: libc::msghdr,
}

impl SendBuffer {
    fn new() -> *mut Self {
        let mut instance = Box::new(Self {
            name_v4: libc::sockaddr_in {
                sin_family: libc::AF_INET as u16,
                sin_port: 0,
                sin_addr: libc::in_addr { s_addr: 0 },
                sin_zero: [0; 8],
            },
            name_v6: libc::sockaddr_in6 {
                sin6_family: libc::AF_INET6 as u16,
                sin6_port: 0,
                sin6_flowinfo: 0,
                sin6_addr: libc::in6_addr { s6_addr: [0; 16] },
                sin6_scope_id: 0,
            },
            bytes: [0; RESPONSE_BUF_LEN],
            iovec: libc::iovec {
                iov_base: null_mut(),
                iov_len: 0,
            },
            msghdr: unsafe { MaybeUninit::<libc::msghdr>::zeroed().assume_init() },
        });

        instance.iovec.iov_base = addr_of_mut!(instance.bytes) as *mut libc::c_void;
        instance.iovec.iov_len = instance.bytes.len();

        instance.msghdr.msg_iov = addr_of_mut!(instance.iovec);
        instance.msghdr.msg_iovlen = 1;

        // Set IPv4 initially. Will be overridden with each prepare_entry call
        instance.msghdr.msg_name = addr_of_mut!(instance.name_v4) as *mut libc::c_void;
        instance.msghdr.msg_namelen = core::mem::size_of::<libc::sockaddr_in>() as u32;

        Box::into_raw(instance)
    }

    fn prepare_entry(
        &mut self,
        response: Response,
        addr: CanonicalSocketAddr,
        send_to_ipv4_socket: bool,
        metadata: &mut SendBufferMetadata,
    ) -> Result<io_uring::squeue::Entry, Error> {
        let entry_fd = if send_to_ipv4_socket {
            metadata.receiver_is_ipv4 = true;

            let addr = if let Some(SocketAddr::V4(addr)) = addr.get_ipv4() {
                addr
            } else {
                panic!("ipv6 address in ipv4 mode");
            };

            self.name_v4.sin_port = addr.port().to_be();
            self.name_v4.sin_addr.s_addr = u32::from(*addr.ip()).to_be();
            self.msghdr.msg_name = addr_of_mut!(self.name_v4) as *mut libc::c_void;
            self.msghdr.msg_namelen = core::mem::size_of::<libc::sockaddr_in>() as u32;

            SOCKET_IDENTIFIER_V4
        } else {
            // Set receiver protocol type before calling addr.get_ipv6_mapped()
            metadata.receiver_is_ipv4 = addr.is_ipv4();

            let addr = if let SocketAddr::V6(addr) = addr.get_ipv6_mapped() {
                addr
            } else {
                panic!("ipv4 address when ipv6 or ipv6-mapped address expected");
            };

            self.name_v6.sin6_port = addr.port().to_be();
            self.name_v6.sin6_addr.s6_addr = addr.ip().octets();
            self.msghdr.msg_name = addr_of_mut!(self.name_v6) as *mut libc::c_void;
            self.msghdr.msg_namelen = core::mem::size_of::<libc::sockaddr_in6>() as u32;

            SOCKET_IDENTIFIER_V6
        };

        let mut cursor = Cursor::new(&mut self.bytes[..]);

        match response.write_bytes(&mut cursor) {
            Ok(()) => {
                self.iovec.iov_len = cursor.position() as usize;

                metadata.response_type = ResponseType::from_response(&response);

                Ok(SendMsg::new(entry_fd, addr_of_mut!(self.msghdr)).build())
            }
            Err(err) => Err(Error::SerializationFailed(err)),
        }
    }
}

#[derive(Debug)]
struct SendBufferMetadata {
    free: bool,
    /// Only used for statistics
    receiver_is_ipv4: bool,
    /// Only used for statistics
    response_type: ResponseType,
}

impl Default for SendBufferMetadata {
    fn default() -> Self {
        Self {
            free: true,
            receiver_is_ipv4: true,
            response_type: Default::default(),
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum ResponseType {
    #[default]
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
