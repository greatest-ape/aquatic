use std::io::{Cursor, ErrorKind};
use std::marker::PhantomData;
use std::sync::atomic::Ordering;

use anyhow::Context;
use mio::net::UdpSocket;
use socket2::{Domain, Protocol, Type};

use aquatic_common::{privileges::PrivilegeDropper, CanonicalSocketAddr};
use aquatic_udp_protocol::*;

use crate::config::Config;

use super::{WorkerSharedData, EXTRA_PACKET_SIZE_IPV4, EXTRA_PACKET_SIZE_IPV6};

pub trait IpVersion {
    fn is_v4() -> bool;
}

#[derive(Clone, Copy, Debug)]
pub struct Ipv4;

impl IpVersion for Ipv4 {
    fn is_v4() -> bool {
        true
    }
}

#[derive(Clone, Copy, Debug)]
pub struct Ipv6;

impl IpVersion for Ipv6 {
    fn is_v4() -> bool {
        false
    }
}

pub struct Socket<V> {
    pub socket: UdpSocket,
    opt_resend_buffer: Option<Vec<(CanonicalSocketAddr, Response)>>,
    phantom_data: PhantomData<V>,
}

impl Socket<Ipv4> {
    pub fn create(config: &Config, priv_dropper: PrivilegeDropper) -> anyhow::Result<Self> {
        let socket = socket2::Socket::new(Domain::IPV4, Type::DGRAM, Some(Protocol::UDP))?;

        socket
            .set_reuse_port(true)
            .with_context(|| "socket: set reuse port")?;
        socket
            .set_nonblocking(true)
            .with_context(|| "socket: set nonblocking")?;

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

        socket
            .bind(&config.network.address_ipv4.into())
            .with_context(|| format!("socket: bind to {}", config.network.address_ipv4))?;

        priv_dropper.after_socket_creation()?;

        let mut s = Self {
            socket: UdpSocket::from_std(::std::net::UdpSocket::from(socket)),
            opt_resend_buffer: None,
            phantom_data: Default::default(),
        };

        if config.network.resend_buffer_max_len > 0 {
            s.opt_resend_buffer = Some(Vec::new());
        }

        Ok(s)
    }
}

impl Socket<Ipv6> {
    pub fn create(config: &Config, priv_dropper: PrivilegeDropper) -> anyhow::Result<Self> {
        let socket = socket2::Socket::new(Domain::IPV6, Type::DGRAM, Some(Protocol::UDP))?;

        if config.network.set_only_ipv6 {
            socket
                .set_only_v6(true)
                .with_context(|| "socket: set only ipv6")?;
        }
        socket
            .set_reuse_port(true)
            .with_context(|| "socket: set reuse port")?;
        socket
            .set_nonblocking(true)
            .with_context(|| "socket: set nonblocking")?;

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

        socket
            .bind(&config.network.address_ipv6.into())
            .with_context(|| format!("socket: bind to {}", config.network.address_ipv6))?;

        priv_dropper.after_socket_creation()?;

        let mut s = Self {
            socket: UdpSocket::from_std(::std::net::UdpSocket::from(socket)),
            opt_resend_buffer: None,
            phantom_data: Default::default(),
        };

        if config.network.resend_buffer_max_len > 0 {
            s.opt_resend_buffer = Some(Vec::new());
        }

        Ok(s)
    }
}

impl<V: IpVersion> Socket<V> {
    pub fn read_and_handle_requests(&mut self, shared: &mut WorkerSharedData) {
        let max_scrape_torrents = shared.config.protocol.max_scrape_torrents;

        loop {
            match self.socket.recv_from(&mut shared.buffer[..]) {
                Ok((bytes_read, src)) => {
                    let src_port = src.port();
                    let src = CanonicalSocketAddr::new(src);

                    // Use canonical address for statistics
                    let opt_statistics = if shared.config.statistics.active() {
                        if src.is_ipv4() {
                            let statistics = &shared.statistics.ipv4;

                            statistics
                                .bytes_received
                                .fetch_add(bytes_read + EXTRA_PACKET_SIZE_IPV4, Ordering::Relaxed);

                            Some(statistics)
                        } else {
                            let statistics = &shared.statistics.ipv6;

                            statistics
                                .bytes_received
                                .fetch_add(bytes_read + EXTRA_PACKET_SIZE_IPV6, Ordering::Relaxed);

                            Some(statistics)
                        }
                    } else {
                        None
                    };

                    if src_port == 0 {
                        ::log::debug!("Ignored request because source port is zero");

                        continue;
                    }

                    match Request::parse_bytes(&shared.buffer[..bytes_read], max_scrape_torrents) {
                        Ok(request) => {
                            if let Some(statistics) = opt_statistics {
                                statistics.requests.fetch_add(1, Ordering::Relaxed);
                            }

                            if let Some(response) = shared.handle_request(request, src) {
                                self.send_response(shared, src, response, false);
                            }
                        }
                        Err(RequestParseError::Sendable {
                            connection_id,
                            transaction_id,
                            err,
                        }) if shared.validator.connection_id_valid(src, connection_id) => {
                            let response = ErrorResponse {
                                transaction_id,
                                message: err.into(),
                            };

                            self.send_response(shared, src, Response::Error(response), false);

                            ::log::debug!("request parse error (sent error response): {:?}", err);
                        }
                        Err(err) => {
                            ::log::debug!(
                                "request parse error (didn't send error response): {:?}",
                                err
                            );
                        }
                    };
                }
                Err(err) if err.kind() == ErrorKind::WouldBlock => {
                    break;
                }
                Err(err) => {
                    ::log::warn!("recv_from error: {:#}", err);
                }
            }
        }
    }
    pub fn send_response(
        &mut self,
        shared: &mut WorkerSharedData,
        canonical_addr: CanonicalSocketAddr,
        response: Response,
        disable_resend_buffer: bool,
    ) {
        let mut buffer = Cursor::new(&mut shared.buffer[..]);

        if let Err(err) = response.write_bytes(&mut buffer) {
            ::log::error!("failed writing response to buffer: {:#}", err);

            return;
        }

        let bytes_written = buffer.position() as usize;

        let addr = if V::is_v4() {
            canonical_addr
                .get_ipv4()
                .expect("found peer ipv6 address while running bound to ipv4 address")
        } else {
            canonical_addr.get_ipv6_mapped()
        };

        match self
            .socket
            .send_to(&buffer.into_inner()[..bytes_written], addr)
        {
            Ok(bytes_sent) if shared.config.statistics.active() => {
                let stats = if canonical_addr.is_ipv4() {
                    let stats = &shared.statistics.ipv4;

                    stats
                        .bytes_sent
                        .fetch_add(bytes_sent + EXTRA_PACKET_SIZE_IPV4, Ordering::Relaxed);

                    stats
                } else {
                    let stats = &shared.statistics.ipv6;

                    stats
                        .bytes_sent
                        .fetch_add(bytes_sent + EXTRA_PACKET_SIZE_IPV6, Ordering::Relaxed);

                    stats
                };

                match response {
                    Response::Connect(_) => {
                        stats.responses_connect.fetch_add(1, Ordering::Relaxed);
                    }
                    Response::AnnounceIpv4(_) | Response::AnnounceIpv6(_) => {
                        stats.responses_announce.fetch_add(1, Ordering::Relaxed);
                    }
                    Response::Scrape(_) => {
                        stats.responses_scrape.fetch_add(1, Ordering::Relaxed);
                    }
                    Response::Error(_) => {
                        stats.responses_error.fetch_add(1, Ordering::Relaxed);
                    }
                }
            }
            Ok(_) => (),
            Err(err) => match self.opt_resend_buffer.as_mut() {
                Some(resend_buffer)
                    if !disable_resend_buffer
                        && ((err.raw_os_error() == Some(libc::ENOBUFS))
                            || (err.kind() == ErrorKind::WouldBlock)) =>
                {
                    if resend_buffer.len() < shared.config.network.resend_buffer_max_len {
                        ::log::debug!("Adding response to resend queue, since sending it to {} failed with: {:#}", addr, err);

                        resend_buffer.push((canonical_addr, response));
                    } else {
                        ::log::warn!("Response resend buffer full, dropping response");
                    }
                }
                _ => {
                    ::log::warn!("Sending response to {} failed: {:#}", addr, err);
                }
            },
        }

        ::log::debug!("send response fn finished");
    }

    /// If resend buffer is enabled, send any responses in it
    pub fn resend_failed(&mut self, shared: &mut WorkerSharedData) {
        if self.opt_resend_buffer.is_some() {
            let mut tmp_resend_buffer = Vec::new();

            // Do memory swap shenanigans to get around false positive in
            // borrow checker regarding double mut borrowing of self

            if let Some(resend_buffer) = self.opt_resend_buffer.as_mut() {
                ::std::mem::swap(resend_buffer, &mut tmp_resend_buffer);
            }

            for (addr, response) in tmp_resend_buffer.drain(..) {
                self.send_response(shared, addr, response, true);
            }

            if let Some(resend_buffer) = self.opt_resend_buffer.as_mut() {
                ::std::mem::swap(resend_buffer, &mut tmp_resend_buffer);
            }
        }
    }
}
