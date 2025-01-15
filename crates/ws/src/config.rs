use std::net::SocketAddr;
use std::path::PathBuf;

use aquatic_common::{access_list::AccessListConfig, privileges::PrivilegeConfig};
use serde::Deserialize;

use aquatic_common::cli::LogLevel;
use aquatic_toml_config::TomlConfig;

/// aquatic_ws configuration
///
/// Running behind a reverse proxy is supported, but IPv4 peer requests have
/// to be proxied to IPv4 requests, and IPv6 requests to IPv6 requests.
#[derive(Clone, Debug, PartialEq, TomlConfig, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct Config {
    /// Number of socket workers.
    ///
    /// On servers with 1-7 physical cores, using a worker per core is
    /// recommended. With more cores, using two workers less than the
    /// number of cores is recommended.
    ///
    /// Socket workers receive requests from the socket, parse them and send
    /// them on to the swarm workers. They then receive responses from the
    /// swarm workers, encode them and send them back over the socket.
    pub socket_workers: usize,
    /// Number of swarm workers.
    ///
    /// A single worker is recommended for servers with 1-7 physical cores.
    /// With more cores, using two workers is recommended.
    ///
    /// Swarm workers receive a number of requests from socket workers,
    /// generate responses and send them back to the socket workers.
    pub swarm_workers: usize,
    pub log_level: LogLevel,
    pub network: NetworkConfig,
    pub protocol: ProtocolConfig,
    pub cleaning: CleaningConfig,
    pub privileges: PrivilegeConfig,
    /// Access list configuration
    ///
    /// The file is read on start and when the program receives `SIGUSR1`. If
    /// initial parsing fails, the program exits. Later failures result in in
    /// emitting of an error-level log message, while successful updates of the
    /// access list result in emitting of an info-level log message.
    pub access_list: AccessListConfig,
    #[cfg(feature = "metrics")]
    pub metrics: MetricsConfig,
}

impl Default for Config {
    fn default() -> Self {
        Self {
            socket_workers: 1,
            swarm_workers: 1,
            log_level: LogLevel::default(),
            network: NetworkConfig::default(),
            protocol: ProtocolConfig::default(),
            cleaning: CleaningConfig::default(),
            privileges: PrivilegeConfig::default(),
            access_list: AccessListConfig::default(),
            #[cfg(feature = "metrics")]
            metrics: Default::default(),
        }
    }
}

impl aquatic_common::cli::Config for Config {
    fn get_log_level(&self) -> Option<LogLevel> {
        Some(self.log_level)
    }
}

#[derive(Clone, Debug, PartialEq, TomlConfig, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct NetworkConfig {
    /// Bind to this address
    ///
    /// When providing an IPv4 style address, only IPv4 traffic will be
    /// handled. Examples:
    /// - "0.0.0.0:3000" binds to port 3000 on all network interfaces
    /// - "127.0.0.1:3000" binds to port 3000 on the loopback interface
    ///   (localhost)
    ///
    /// When it comes to IPv6-style addresses, behaviour is more complex and
    /// differs between operating systems. On Linux, to accept both IPv4 and
    /// IPv6 traffic on any interface, use "[::]:3000". Set the "only_ipv6"
    /// flag below to limit traffic to IPv6. To bind to the loopback interface
    /// and only accept IPv6 packets, use "[::1]:3000" and set the only_ipv6
    /// flag. Receiving both IPv4 and IPv6 traffic on loopback is currently
    /// not supported. For other operating systems, please refer to their
    /// respective documentation.
    pub address: SocketAddr,
    /// Only allow access over IPv6
    pub only_ipv6: bool,
    /// Maximum number of pending TCP connections
    pub tcp_backlog: i32,

    /// Enable TLS
    ///
    /// The TLS files are read on start and when the program receives `SIGUSR1`.
    /// If initial parsing fails, the program exits. Later failures result in
    /// in emitting of an error-level log message, while successful updates
    /// result in emitting of an info-level log message. Updates only affect
    /// new connections.
    pub enable_tls: bool,
    /// Path to TLS certificate (DER-encoded X.509)
    pub tls_certificate_path: PathBuf,
    /// Path to TLS private key (DER-encoded ASN.1 in PKCS#8 or PKCS#1 format)
    pub tls_private_key_path: PathBuf,

    pub websocket_max_message_size: usize,
    pub websocket_max_frame_size: usize,
    pub websocket_write_buffer_size: usize,

    /// Return a HTTP 200 Ok response when receiving GET /health. Can not be
    /// combined with enable_tls.
    pub enable_http_health_checks: bool,
}

impl Default for NetworkConfig {
    fn default() -> Self {
        Self {
            address: SocketAddr::from(([0, 0, 0, 0], 3000)),
            only_ipv6: false,
            tcp_backlog: 1024,

            enable_tls: false,
            tls_certificate_path: "".into(),
            tls_private_key_path: "".into(),

            websocket_max_message_size: 64 * 1024,
            websocket_max_frame_size: 16 * 1024,
            websocket_write_buffer_size: 8 * 1024,

            enable_http_health_checks: false,
        }
    }
}

#[derive(Clone, Debug, PartialEq, TomlConfig, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct ProtocolConfig {
    /// Maximum number of torrents to accept in scrape request
    pub max_scrape_torrents: usize,
    /// Maximum number of offers to accept in announce request
    pub max_offers: usize,
    /// Ask peers to announce this often (seconds)
    pub peer_announce_interval: usize,
}

impl Default for ProtocolConfig {
    fn default() -> Self {
        Self {
            max_scrape_torrents: 255,
            max_offers: 10,
            peer_announce_interval: 120,
        }
    }
}

#[derive(Clone, Debug, PartialEq, TomlConfig, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct CleaningConfig {
    /// Clean peers this often (seconds)
    pub torrent_cleaning_interval: u64,
    /// Remove peers that have not announced for this long (seconds)
    pub max_peer_age: u32,
    /// Require that offers are answered to withing this period (seconds)
    pub max_offer_age: u32,
    // Clean connections this often (seconds)
    pub connection_cleaning_interval: u64,
    /// Close connections if no responses have been sent to them for this long (seconds)
    pub max_connection_idle: u32,
    /// After updating TLS certificates, close connections running with
    /// previous certificates after this long (seconds)
    ///
    /// Countdown starts at next connection cleaning.
    pub close_after_tls_update_grace_period: u32,
}

impl Default for CleaningConfig {
    fn default() -> Self {
        Self {
            torrent_cleaning_interval: 30,
            max_peer_age: 180,
            max_offer_age: 120,
            max_connection_idle: 180,
            connection_cleaning_interval: 30,
            close_after_tls_update_grace_period: 60 * 60 * 60,
        }
    }
}

#[cfg(feature = "metrics")]
#[derive(Clone, Debug, PartialEq, TomlConfig, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct MetricsConfig {
    /// Run a prometheus endpoint
    pub run_prometheus_endpoint: bool,
    /// Address to run prometheus endpoint on
    pub prometheus_endpoint_address: SocketAddr,
    /// Update metrics for torrent count this often (seconds)
    pub torrent_count_update_interval: u64,
    /// Serve information on peer clients
    ///
    /// Expect a certain CPU hit
    pub peer_clients: bool,
    /// Serve information on all peer id prefixes
    ///
    /// Requires `peer_clients` to be activated.
    ///
    /// Expect a certain CPU hit
    pub peer_id_prefixes: bool,
}

#[cfg(feature = "metrics")]
impl Default for MetricsConfig {
    fn default() -> Self {
        Self {
            run_prometheus_endpoint: false,
            prometheus_endpoint_address: SocketAddr::from(([0, 0, 0, 0], 9000)),
            torrent_count_update_interval: 10,
            peer_clients: false,
            peer_id_prefixes: false,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::Config;

    ::aquatic_toml_config::gen_serialize_deserialize_test!(Config);
}
