mod mio;
#[cfg(all(target_os = "linux", feature = "io-uring"))]
mod uring;
mod validator;

use anyhow::Context;
use aquatic_common::privileges::PrivilegeDropper;
use crossbeam_channel::Sender;
use socket2::{Domain, Protocol, Socket, Type};

use crate::{
    common::{
        CachePaddedArc, IpVersionStatistics, SocketWorkerStatistics, State, StatisticsMessage,
    },
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

pub fn run_socket_worker(
    config: Config,
    shared_state: State,
    statistics: CachePaddedArc<IpVersionStatistics<SocketWorkerStatistics>>,
    statistics_sender: Sender<StatisticsMessage>,
    validator: ConnectionValidator,
    priv_dropper: PrivilegeDropper,
) -> anyhow::Result<()> {
    #[cfg(all(target_os = "linux", feature = "io-uring"))]
    if config.network.use_io_uring {
        self::uring::supported_on_current_kernel().context("check for io_uring compatibility")?;

        return self::uring::SocketWorker::run(
            config,
            shared_state,
            statistics,
            statistics_sender,
            validator,
            priv_dropper,
        );
    }

    self::mio::SocketWorker::run(
        config,
        shared_state,
        statistics,
        statistics_sender,
        validator,
        priv_dropper,
    )
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
