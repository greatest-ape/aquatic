use std::net::SocketAddr;

use serde::{Deserialize, Serialize};

use aquatic_common::cli::LogLevel;
#[cfg(feature = "cpu-pinning")]
use aquatic_common::cpu_pinning::desc::CpuPinningConfigDesc;
use aquatic_toml_config::TomlConfig;

/// aquatic_udp_load_test configuration
#[derive(Clone, Debug, PartialEq, TomlConfig, Deserialize, Serialize)]
#[serde(default, deny_unknown_fields)]
pub struct Config {
    /// Server address
    ///
    /// If you want to send IPv4 requests to a IPv4+IPv6 tracker, put an IPv4
    /// address here.
    pub server_address: SocketAddr,
    pub log_level: LogLevel,
    /// Number of workers sending requests
    pub workers: u8,
    /// Run duration (quit and generate report after this many seconds)
    pub duration: usize,
    /// Only report summary for the last N seconds of run
    ///
    /// 0 = include whole run
    pub summarize_last: usize,
    /// Display extra statistics
    pub extra_statistics: bool,
    pub network: NetworkConfig,
    pub requests: RequestConfig,
    #[cfg(feature = "cpu-pinning")]
    pub cpu_pinning: CpuPinningConfigDesc,
}

impl Default for Config {
    fn default() -> Self {
        Self {
            server_address: "127.0.0.1:3000".parse().unwrap(),
            log_level: LogLevel::Error,
            workers: 1,
            duration: 0,
            summarize_last: 0,
            extra_statistics: true,
            network: NetworkConfig::default(),
            requests: RequestConfig::default(),
            #[cfg(feature = "cpu-pinning")]
            cpu_pinning: Default::default(),
        }
    }
}

impl aquatic_common::cli::Config for Config {
    fn get_log_level(&self) -> Option<aquatic_common::cli::LogLevel> {
        Some(self.log_level)
    }
}

#[derive(Clone, Debug, PartialEq, TomlConfig, Deserialize, Serialize)]
#[serde(default, deny_unknown_fields)]
pub struct NetworkConfig {
    /// True means bind to one localhost IP per socket.
    ///
    /// The point of multiple IPs is to cause a better distribution
    /// of requests to servers with SO_REUSEPORT option.
    ///
    /// Setting this to true can cause issues on macOS.
    pub multiple_client_ipv4s: bool,
    /// Number of sockets to open per worker
    pub sockets_per_worker: u8,
    /// Size of socket recv buffer. Use 0 for OS default.
    ///
    /// This setting can have a big impact on dropped packages. It might
    /// require changing system defaults. Some examples of commands to set
    /// values for different operating systems:
    ///
    /// macOS:
    /// $ sudo sysctl net.inet.udp.recvspace=8000000
    ///
    /// Linux:
    /// $ sudo sysctl -w net.core.rmem_max=8000000
    /// $ sudo sysctl -w net.core.rmem_default=8000000
    pub recv_buffer: usize,
}

impl Default for NetworkConfig {
    fn default() -> Self {
        Self {
            multiple_client_ipv4s: true,
            sockets_per_worker: 4,
            recv_buffer: 8_000_000,
        }
    }
}

#[derive(Clone, Debug, PartialEq, TomlConfig, Deserialize, Serialize)]
#[serde(default, deny_unknown_fields)]
pub struct RequestConfig {
    /// Number of torrents to simulate
    pub number_of_torrents: usize,
    /// Number of peers to simulate
    pub number_of_peers: usize,
    /// Maximum number of torrents to ask about in scrape requests
    pub scrape_max_torrents: usize,
    /// Ask for this number of peers in announce requests
    pub announce_peers_wanted: i32,
    /// Probability that a generated request is a connect request as part
    /// of sum of the various weight arguments.
    pub weight_connect: usize,
    /// Probability that a generated request is a announce request, as part
    /// of sum of the various weight arguments.
    pub weight_announce: usize,
    /// Probability that a generated request is a scrape request, as part
    /// of sum of the various weight arguments.
    pub weight_scrape: usize,
    /// Probability that a generated peer is a seeder
    pub peer_seeder_probability: f64,
}

impl Default for RequestConfig {
    fn default() -> Self {
        Self {
            number_of_torrents: 1_000_000,
            number_of_peers: 2_000_000,
            scrape_max_torrents: 10,
            announce_peers_wanted: 30,
            weight_connect: 50,
            weight_announce: 50,
            weight_scrape: 1,
            peer_seeder_probability: 0.75,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::Config;

    ::aquatic_toml_config::gen_serialize_deserialize_test!(Config);
}
