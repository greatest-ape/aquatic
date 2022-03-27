use std::{net::SocketAddr, path::PathBuf};

use aquatic_common::{access_list::AccessListConfig, privileges::PrivilegeConfig};
use serde::Deserialize;

use aquatic_cli_helpers::LogLevel;
use aquatic_toml_config::TomlConfig;

/// aquatic_udp configuration
#[derive(Clone, Debug, PartialEq, TomlConfig, Deserialize)]
#[serde(default)]
pub struct Config {
    /// Socket workers receive requests from the socket, parse them and send
    /// them on to the request workers. They then receive responses from the
    /// request workers, encode them and send them back over the socket.
    pub socket_workers: usize,
    /// Request workers receive a number of requests from socket workers,
    /// generate responses and send them back to the socket workers.
    pub request_workers: usize,
    pub log_level: LogLevel,
    /// Maximum number of items in each channel passing requests/responses
    /// between workers. A value of zero means that the channel will be of
    /// unbounded size.
    pub worker_channel_size: usize,
    /// How long to block waiting for requests in request workers. Higher
    /// values means that with zero traffic, the worker will not unnecessarily
    /// cause the CPU to wake up as often. However, high values (something like
    /// larger than 1000) combined with very low traffic can cause delays
    /// in torrent cleaning.
    pub request_channel_recv_timeout_ms: u64,
    pub network: NetworkConfig,
    pub protocol: ProtocolConfig,
    pub statistics: StatisticsConfig,
    pub cleaning: CleaningConfig,
    pub privileges: PrivilegeConfig,
    pub access_list: AccessListConfig,
    #[cfg(feature = "cpu-pinning")]
    pub cpu_pinning: aquatic_common::cpu_pinning::CpuPinningConfig,
}

impl Default for Config {
    fn default() -> Self {
        Self {
            socket_workers: 1,
            request_workers: 1,
            log_level: LogLevel::Error,
            worker_channel_size: 0,
            request_channel_recv_timeout_ms: 100,
            network: NetworkConfig::default(),
            protocol: ProtocolConfig::default(),
            statistics: StatisticsConfig::default(),
            cleaning: CleaningConfig::default(),
            privileges: PrivilegeConfig::default(),
            access_list: AccessListConfig::default(),
            #[cfg(feature = "cpu-pinning")]
            cpu_pinning: Default::default(),
        }
    }
}

impl aquatic_cli_helpers::Config for Config {
    fn get_log_level(&self) -> Option<LogLevel> {
        Some(self.log_level)
    }
}

#[derive(Clone, Debug, PartialEq, TomlConfig, Deserialize)]
#[serde(default)]
pub struct NetworkConfig {
    /// Bind to this address
    pub address: SocketAddr,
    /// Only allow access over IPv6
    pub only_ipv6: bool,
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
    pub socket_recv_buffer_size: usize,
    pub poll_event_capacity: usize,
    pub poll_timeout_ms: u64,
}

impl NetworkConfig {
    pub fn ipv4_active(&self) -> bool {
        self.address.is_ipv4() || !self.only_ipv6
    }
    pub fn ipv6_active(&self) -> bool {
        self.address.is_ipv6()
    }
}

impl Default for NetworkConfig {
    fn default() -> Self {
        Self {
            address: SocketAddr::from(([0, 0, 0, 0], 3000)),
            only_ipv6: false,
            socket_recv_buffer_size: 4096 * 128,
            poll_event_capacity: 4096,
            poll_timeout_ms: 50,
        }
    }
}

#[derive(Clone, Debug, PartialEq, TomlConfig, Deserialize)]
#[serde(default)]
pub struct ProtocolConfig {
    /// Maximum number of torrents to accept in scrape request
    pub max_scrape_torrents: u8,
    /// Maximum number of peers to return in announce response
    pub max_response_peers: usize,
    /// Ask peers to announce this often (seconds)
    pub peer_announce_interval: i32,
}

impl Default for ProtocolConfig {
    fn default() -> Self {
        Self {
            max_scrape_torrents: 70,
            max_response_peers: 50,
            peer_announce_interval: 60 * 15,
        }
    }
}

#[derive(Clone, Debug, PartialEq, TomlConfig, Deserialize)]
#[serde(default)]
pub struct StatisticsConfig {
    /// Collect and print/write statistics this often (seconds)
    pub interval: u64,
    /// Print statistics to standard output
    pub print_to_stdout: bool,
    /// Save statistics as HTML to a file
    pub write_html_to_file: bool,
    /// Path to save HTML file to
    pub html_file_path: PathBuf,
    /// Report response latencies
    pub latencies: bool,
}

impl StatisticsConfig {
    pub fn active(&self) -> bool {
        (self.interval != 0) & (self.print_to_stdout | self.write_html_to_file)
    }
}

impl Default for StatisticsConfig {
    fn default() -> Self {
        Self {
            interval: 5,
            print_to_stdout: false,
            write_html_to_file: false,
            html_file_path: "tmp/statistics.html".into(),
            latencies: true,
        }
    }
}

#[derive(Clone, Debug, PartialEq, TomlConfig, Deserialize)]
#[serde(default)]
pub struct CleaningConfig {
    /// Clean connections this often (seconds)
    pub connection_cleaning_interval: u64,
    /// Clean torrents this often (seconds)
    pub torrent_cleaning_interval: u64,
    /// Clean pending scrape responses this often (seconds)
    ///
    /// In regular operation, there should be no pending scrape responses
    /// lingering for a long time. However, the cleaning also returns unused
    /// allocated memory to the OS, so the interval can be configured here.
    pub pending_scrape_cleaning_interval: u64,
    /// Remove connections that are older than this (seconds)
    pub max_connection_age: u64,
    /// Remove peers who have not announced for this long (seconds)
    pub max_peer_age: u64,
    /// Remove pending scrape responses that have not been returned from request
    /// workers for this long (seconds)
    pub max_pending_scrape_age: u64,
}

impl Default for CleaningConfig {
    fn default() -> Self {
        Self {
            connection_cleaning_interval: 60,
            torrent_cleaning_interval: 60 * 2,
            pending_scrape_cleaning_interval: 60 * 10,
            max_connection_age: 60 * 2,
            max_peer_age: 60 * 20,
            max_pending_scrape_age: 60,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::Config;

    ::aquatic_toml_config::gen_serialize_deserialize_test!(Config);
}
