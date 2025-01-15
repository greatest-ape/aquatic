mod mio;
#[cfg(all(target_os = "linux", feature = "io-uring"))]
mod uring;
mod validator;

use aquatic_common::privileges::PrivilegeDropper;
use crossbeam_channel::Sender;

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
    priv_droppers: Vec<PrivilegeDropper>,
) -> anyhow::Result<()> {
    #[cfg(all(target_os = "linux", feature = "io-uring"))]
    if config.network.use_io_uring {
        use anyhow::Context;

        self::uring::supported_on_current_kernel().context("check for io_uring compatibility")?;

        return self::uring::SocketWorker::run(
            config,
            shared_state,
            statistics,
            statistics_sender,
            validator,
            priv_droppers,
        );
    }

    self::mio::run(
        config,
        shared_state,
        statistics,
        statistics_sender,
        validator,
        priv_droppers,
    )
}
