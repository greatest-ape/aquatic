mod mio;
mod storage;
#[cfg(all(target_os = "linux", feature = "io-uring"))]
mod uring;
mod validator;

use anyhow::Context;
use aquatic_common::{privileges::PrivilegeDropper, PanicSentinel, ServerStartInstant};
use socket2::{Domain, Protocol, Socket, Type};

use crate::{
    common::{ConnectedRequestSender, ConnectedResponseReceiver, State},
    config::Config,
};

pub use self::validator::ConnectionValidator;

#[cfg(all(not(target_os = "linux"), feature = "io-uring"))]
compile_error!("io_uring feature is only supported on Linux");

/// Bytes of data transmitted when sending an IPv4 UDP packet, in addition to payload size
///
/// Consists of:
/// - 8 bit ethernet frame
/// - 14 + 4 bit MAC header and checksum
/// - 20 bit IPv4 header
/// - 8 bit udp header
const EXTRA_PACKET_SIZE_IPV4: usize = 8 + 18 + 20 + 8;

/// Bytes of data transmitted when sending an IPv4 UDP packet, in addition to payload size
///
/// Consists of:
/// - 8 bit ethernet frame
/// - 14 + 4 bit MAC header and checksum
/// - 40 bit IPv6 header
/// - 8 bit udp header
const EXTRA_PACKET_SIZE_IPV6: usize = 8 + 18 + 40 + 8;

#[allow(clippy::too_many_arguments)]
pub fn run_socket_worker(
    sentinel: PanicSentinel,
    shared_state: State,
    config: Config,
    validator: ConnectionValidator,
    server_start_instant: ServerStartInstant,
    request_sender: ConnectedRequestSender,
    response_receiver: ConnectedResponseReceiver,
    priv_dropper: PrivilegeDropper,
) {
    #[cfg(all(target_os = "linux", feature = "io-uring"))]
    match self::uring::supported_on_current_kernel() {
        Ok(()) => {
            self::uring::SocketWorker::run(
                sentinel,
                shared_state,
                config,
                validator,
                server_start_instant,
                request_sender,
                response_receiver,
                priv_dropper,
            );

            return;
        }
        Err(err) => {
            ::log::warn!(
                "Falling back to mio because of lacking kernel io_uring support: {:#}",
                err
            );
        }
    }

    self::mio::SocketWorker::run(
        sentinel,
        shared_state,
        config,
        validator,
        server_start_instant,
        request_sender,
        response_receiver,
        priv_dropper,
    );
}

fn create_socket(
    config: &Config,
    priv_dropper: PrivilegeDropper,
) -> anyhow::Result<::std::net::UdpSocket> {
    let socket = if config.network.address.is_ipv4() {
        Socket::new(Domain::IPV4, Type::DGRAM, Some(Protocol::UDP))?
    } else {
        Socket::new(Domain::IPV6, Type::DGRAM, Some(Protocol::UDP))?
    };

    if config.network.only_ipv6 {
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
        .bind(&config.network.address.into())
        .with_context(|| format!("socket: bind to {}", config.network.address))?;

    priv_dropper.after_socket_creation()?;

    Ok(socket.into())
}
