use std::net::SocketAddr;
use std::sync::{atomic::AtomicUsize, Arc};

use aquatic_cli_helpers::LogLevel;
#[cfg(feature = "cpu-pinning")]
use aquatic_common::cpu_pinning::CpuPinningConfig;
use hashbrown::HashMap;
use serde::{Deserialize, Serialize};

use aquatic_udp_protocol::*;

#[derive(Debug, PartialEq, Eq, Hash, Clone, Copy)]
pub struct ThreadId(pub u8);

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(default)]
pub struct Config {
    /// Server address
    ///
    /// If you want to send IPv4 requests to a IPv4+IPv6 tracker, put an IPv4
    /// address here.
    pub server_address: SocketAddr,
    pub log_level: LogLevel,
    pub workers: u8,
    /// Run duration (quit and generate report after this many seconds)
    pub duration: usize,
    pub network: NetworkConfig,
    pub handler: HandlerConfig,
    #[cfg(feature = "cpu-pinning")]
    pub cpu_pinning: CpuPinningConfig,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(default)]
pub struct NetworkConfig {
    /// True means bind to one localhost IP per socket.
    ///
    /// The point of multiple IPs is to cause a better distribution
    /// of requests to servers with SO_REUSEPORT option.
    ///
    /// Setting this to true can cause issues on macOS.
    pub multiple_client_ipv4s: bool,
    /// Number of first client port
    pub first_port: u16,
    /// Socket worker poll timeout in microseconds
    pub poll_timeout: u64,
    /// Socket worker polling event number
    pub poll_event_capacity: usize,
    /// Size of socket recv buffer. Use 0 for OS default.
    ///
    /// This setting can have a big impact on dropped packages. It might
    /// require changing system defaults. Some examples of commands to set
    /// recommended values for different operating systems:
    ///
    /// macOS:
    /// $ sudo sysctl net.inet.udp.recvspace=6000000
    /// $ sudo sysctl net.inet.udp.maxdgram=500000 # Not necessary, but recommended
    /// $ sudo sysctl kern.ipc.maxsockbuf=8388608 # Not necessary, but recommended
    ///
    /// Linux:
    /// $ sudo sysctl -w net.core.rmem_max=104857600
    /// $ sudo sysctl -w net.core.rmem_default=104857600
    pub recv_buffer: usize,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(default)]
pub struct HandlerConfig {
    /// Number of torrents to simulate
    pub number_of_torrents: usize,
    /// Maximum number of torrents to ask about in scrape requests
    pub scrape_max_torrents: usize,
    /// Probability that a generated request is a connect request as part
    /// of sum of the various weight arguments.
    pub weight_connect: usize,
    /// Probability that a generated request is a announce request, as part
    /// of sum of the various weight arguments.
    pub weight_announce: usize,
    /// Probability that a generated request is a scrape request, as part
    /// of sum of the various weight arguments.
    pub weight_scrape: usize,
    /// Pareto shape
    ///
    /// Fake peers choose torrents according to Pareto distribution.
    pub torrent_selection_pareto_shape: f64,
    /// Probability that a generated peer is a seeder
    pub peer_seeder_probability: f64,
}

impl Default for Config {
    fn default() -> Self {
        Self {
            server_address: "127.0.0.1:3000".parse().unwrap(),
            log_level: LogLevel::Error,
            workers: 1,
            duration: 0,
            network: NetworkConfig::default(),
            handler: HandlerConfig::default(),
            #[cfg(feature = "cpu-pinning")]
            cpu_pinning: CpuPinningConfig::default_for_load_test(),
        }
    }
}

impl Default for NetworkConfig {
    fn default() -> Self {
        Self {
            multiple_client_ipv4s: true,
            first_port: 45_000,
            poll_timeout: 276,
            poll_event_capacity: 2_877,
            recv_buffer: 6_000_000,
        }
    }
}

impl Default for HandlerConfig {
    fn default() -> Self {
        Self {
            number_of_torrents: 10_000,
            peer_seeder_probability: 0.25,
            scrape_max_torrents: 50,
            weight_connect: 0,
            weight_announce: 5,
            weight_scrape: 1,
            torrent_selection_pareto_shape: 2.0,
        }
    }
}

#[derive(PartialEq, Eq, Clone)]
pub struct TorrentPeer {
    pub info_hash: InfoHash,
    pub scrape_hash_indeces: Vec<usize>,
    pub connection_id: ConnectionId,
    pub peer_id: PeerId,
    pub port: Port,
}

pub type TorrentPeerMap = HashMap<TransactionId, TorrentPeer>;

#[derive(Default)]
pub struct Statistics {
    pub requests: AtomicUsize,
    pub response_peers: AtomicUsize,
    pub responses_connect: AtomicUsize,
    pub responses_announce: AtomicUsize,
    pub responses_scrape: AtomicUsize,
    pub responses_error: AtomicUsize,
}

#[derive(Clone)]
pub struct LoadTestState {
    pub info_hashes: Arc<Vec<InfoHash>>,
    pub statistics: Arc<Statistics>,
}

#[derive(PartialEq, Eq, Clone, Copy)]
pub enum RequestType {
    Announce,
    Connect,
    Scrape,
}

#[derive(Default)]
pub struct SocketWorkerLocalStatistics {
    pub requests: usize,
    pub response_peers: usize,
    pub responses_connect: usize,
    pub responses_announce: usize,
    pub responses_scrape: usize,
    pub responses_error: usize,
}
