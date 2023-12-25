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
            network: NetworkConfig::default(),
            requests: RequestConfig::default(),
            #[cfg(feature = "cpu-pinning")]
            cpu_pinning: Default::default(),
        }
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
    /// Number of first client port
    pub first_port: u16,
    /// Socket worker poll timeout in microseconds
    pub poll_timeout: u64,
    /// Size of socket recv buffer. Use 0 for OS default.
    ///
    /// This setting can have a big impact on dropped packages. It might
    /// require changing system defaults. Some examples of commands to set
    /// values for different operating systems:
    ///
    /// macOS:
    /// $ sudo sysctl net.inet.udp.recvspace=6000000
    ///
    /// Linux:
    /// $ sudo sysctl -w net.core.rmem_max=104857600
    /// $ sudo sysctl -w net.core.rmem_default=104857600
    pub recv_buffer: usize,
}

impl Default for NetworkConfig {
    fn default() -> Self {
        Self {
            multiple_client_ipv4s: true,
            first_port: 45_000,
            poll_timeout: 276,
            recv_buffer: 6_000_000,
        }
    }
}

#[derive(Clone, Debug, PartialEq, TomlConfig, Deserialize, Serialize)]
#[serde(default, deny_unknown_fields)]
pub struct RequestConfig {
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
    /// Peers choose torrents according to this Gamma distribution shape
    pub torrent_gamma_shape: f64,
    /// Peers choose torrents according to this Gamma distribution scale
    pub torrent_gamma_scale: f64,
    /// Probability that a generated peer is a seeder
    pub peer_seeder_probability: f64,
    /// Probability that an additional connect request will be sent for each
    /// mio event
    pub additional_request_probability: f32,
}

impl Default for RequestConfig {
    fn default() -> Self {
        Self {
            number_of_torrents: 10_000,
            scrape_max_torrents: 50,
            weight_connect: 0,
            weight_announce: 100,
            weight_scrape: 1,
            torrent_gamma_shape: 0.2,
            torrent_gamma_scale: 100.0,
            peer_seeder_probability: 0.25,
            additional_request_probability: 0.5,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::Config;

    ::aquatic_toml_config::gen_serialize_deserialize_test!(Config);
}
