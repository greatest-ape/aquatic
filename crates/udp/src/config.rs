use std::{
    net::{Ipv4Addr, Ipv6Addr, SocketAddr, SocketAddrV4, SocketAddrV6},
    path::PathBuf,
};

use aquatic_common::{access_list::AccessListConfig, privileges::PrivilegeConfig};
use cfg_if::cfg_if;
use serde::{Deserialize, Serialize};

use aquatic_common::cli::LogLevel;
use aquatic_toml_config::TomlConfig;

/// aquatic_udp configuration
#[derive(Clone, Debug, PartialEq, TomlConfig, Deserialize, Serialize)]
#[serde(default, deny_unknown_fields)]
pub struct Config {
    /// Number of socket workers
    ///
    /// 0 = automatically set to number of available virtual CPUs
    pub socket_workers: usize,
    pub log_level: LogLevel,
    pub network: NetworkConfig,
    pub protocol: ProtocolConfig,
    pub statistics: StatisticsConfig,
    pub cleaning: CleaningConfig,
    pub privileges: PrivilegeConfig,
    /// Access list configuration
    ///
    /// The file is read on start and when the program receives `SIGUSR1`. If
    /// initial parsing fails, the program exits. Later failures result in in
    /// emitting of an error-level log message, while successful updates of the
    /// access list result in emitting of an info-level log message.
    pub access_list: AccessListConfig,
}

impl Default for Config {
    fn default() -> Self {
        Self {
            socket_workers: 1,
            log_level: LogLevel::Error,
            network: NetworkConfig::default(),
            protocol: ProtocolConfig::default(),
            statistics: StatisticsConfig::default(),
            cleaning: CleaningConfig::default(),
            privileges: PrivilegeConfig::default(),
            access_list: AccessListConfig::default(),
        }
    }
}

impl aquatic_common::cli::Config for Config {
    fn get_log_level(&self) -> Option<LogLevel> {
        Some(self.log_level)
    }
}

#[derive(Clone, Debug, PartialEq, TomlConfig, Deserialize, Serialize)]
#[serde(default, deny_unknown_fields)]
pub struct NetworkConfig {
    /// Use IPv4
    pub use_ipv4: bool,
    /// Use IPv6
    pub use_ipv6: bool,
    /// IPv4 address and port
    ///
    /// Examples:
    /// - Use 0.0.0.0:3000 to bind to all interfaces on port 3000
    /// - Use 127.0.0.1:3000 to bind to the loopback interface (localhost) on
    ///   port 3000
    pub address_ipv4: SocketAddrV4,
    /// IPv6 address and port
    ///
    /// Examples:
    /// - Use [::]:3000 to bind to all interfaces on port 3000
    /// - Use [::1]:3000 to bind to the loopback interface (localhost) on
    ///   port 3000
    pub address_ipv6: SocketAddrV6,
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
    pub socket_recv_buffer_size: usize,
    /// Poll timeout in milliseconds (mio backend only)
    pub poll_timeout_ms: u64,
    /// Store this many responses at most for retrying (once) on send failure
    /// (mio backend only)
    ///
    /// Useful on operating systems that do not provide an udp send buffer,
    /// such as FreeBSD. Setting the value to zero disables resending
    /// functionality.
    pub resend_buffer_max_len: usize,
    /// Set flag on IPv6 socket to only accept IPv6 traffic.
    ///
    /// This should typically be set to true unless your OS does not support
    /// double-stack sockets (that is, sockets that receive both IPv4 and IPv6
    /// packets).
    pub set_only_ipv6: bool,
    #[cfg(feature = "io-uring")]
    pub use_io_uring: bool,
    /// Number of ring entries (io_uring backend only)
    ///
    /// Will be rounded to next power of two if not already one.
    #[cfg(feature = "io-uring")]
    pub ring_size: u16,
}

impl NetworkConfig {
    pub fn ipv4_active(&self) -> bool {
        self.use_ipv4
    }
    pub fn ipv6_active(&self) -> bool {
        self.use_ipv6
    }
}

impl Default for NetworkConfig {
    fn default() -> Self {
        Self {
            use_ipv4: true,
            use_ipv6: true,
            address_ipv4: SocketAddrV4::new(Ipv4Addr::UNSPECIFIED, 3000),
            address_ipv6: SocketAddrV6::new(Ipv6Addr::UNSPECIFIED, 3000, 0, 0),
            socket_recv_buffer_size: 8_000_000,
            poll_timeout_ms: 50,
            resend_buffer_max_len: 0,
            set_only_ipv6: true,
            #[cfg(feature = "io-uring")]
            use_io_uring: true,
            #[cfg(feature = "io-uring")]
            ring_size: 128,
        }
    }
}

#[derive(Clone, Debug, PartialEq, TomlConfig, Deserialize, Serialize)]
#[serde(default, deny_unknown_fields)]
pub struct ProtocolConfig {
    /// Maximum number of torrents to allow in scrape request
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
            max_response_peers: 30,
            peer_announce_interval: 60 * 15,
        }
    }
}

#[derive(Clone, Debug, PartialEq, TomlConfig, Deserialize, Serialize)]
#[serde(default, deny_unknown_fields)]
pub struct StatisticsConfig {
    /// Collect and print/write statistics this often (seconds)
    pub interval: u64,
    /// Collect statistics on number of peers per torrent
    ///
    /// Will increase time taken for torrent cleaning.
    pub torrent_peer_histograms: bool,
    /// Collect statistics on peer clients.
    ///
    /// Also, see `prometheus_peer_id_prefixes`.
    ///
    /// Expect a certain CPU hit (maybe 5% higher consumption) and a bit higher
    /// memory use
    pub peer_clients: bool,
    /// Print statistics to standard output
    pub print_to_stdout: bool,
    /// Save statistics as HTML to a file
    pub write_html_to_file: bool,
    /// Path to save HTML file to
    pub html_file_path: PathBuf,
    /// Run a prometheus endpoint
    #[cfg(feature = "prometheus")]
    pub run_prometheus_endpoint: bool,
    /// Address to run prometheus endpoint on
    #[cfg(feature = "prometheus")]
    pub prometheus_endpoint_address: SocketAddr,
    /// Serve information on all peer id prefixes on the prometheus endpoint.
    ///
    /// Requires `peer_clients` to be activated.
    ///
    /// May consume quite a bit of CPU and RAM, since data on every single peer
    /// client will be reported continuously on the endpoint
    #[cfg(feature = "prometheus")]
    pub prometheus_peer_id_prefixes: bool,
}

impl StatisticsConfig {
    cfg_if! {
        if #[cfg(feature = "prometheus")] {
            pub fn active(&self) -> bool {
                (self.interval != 0) &
                    (self.print_to_stdout | self.write_html_to_file | self.run_prometheus_endpoint)
            }
        } else {
            pub fn active(&self) -> bool {
                (self.interval != 0) & (self.print_to_stdout | self.write_html_to_file)
            }
        }
    }
}

impl Default for StatisticsConfig {
    fn default() -> Self {
        Self {
            interval: 5,
            torrent_peer_histograms: false,
            peer_clients: false,
            print_to_stdout: false,
            write_html_to_file: false,
            html_file_path: "tmp/statistics.html".into(),
            #[cfg(feature = "prometheus")]
            run_prometheus_endpoint: false,
            #[cfg(feature = "prometheus")]
            prometheus_endpoint_address: SocketAddr::from(([0, 0, 0, 0], 9000)),
            #[cfg(feature = "prometheus")]
            prometheus_peer_id_prefixes: false,
        }
    }
}

#[derive(Clone, Debug, PartialEq, TomlConfig, Deserialize, Serialize)]
#[serde(default, deny_unknown_fields)]
pub struct CleaningConfig {
    /// Clean torrents this often (seconds)
    pub torrent_cleaning_interval: u64,
    /// Allow clients to use a connection token for this long (seconds)
    pub max_connection_age: u32,
    /// Remove peers who have not announced for this long (seconds)
    pub max_peer_age: u32,
}

impl Default for CleaningConfig {
    fn default() -> Self {
        Self {
            torrent_cleaning_interval: 60 * 2,
            max_connection_age: 60 * 2,
            max_peer_age: 60 * 20,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::Config;

    ::aquatic_toml_config::gen_serialize_deserialize_test!(Config);
}
