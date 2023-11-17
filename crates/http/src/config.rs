use std::{net::SocketAddr, path::PathBuf};

use aquatic_common::{
    access_list::AccessListConfig, cpu_pinning::asc::CpuPinningConfigAsc,
    privileges::PrivilegeConfig,
};
use aquatic_toml_config::TomlConfig;
use serde::{Deserialize, Serialize};

use aquatic_common::cli::LogLevel;

#[derive(Clone, Copy, Debug, PartialEq, Serialize, TomlConfig, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum ReverseProxyPeerIpHeaderFormat {
    #[default]
    LastAddress,
}

/// aquatic_http configuration
#[derive(Clone, Debug, PartialEq, TomlConfig, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct Config {
    /// Number of socket worker. One per physical core is recommended.
    ///
    /// Socket workers receive requests from the socket, parse them and send
    /// them on to the swarm workers. They then receive responses from the
    /// swarm workers, encode them and send them back over the socket.
    pub socket_workers: usize,
    /// Number of swarm workers. One is enough in almost all cases
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
    pub cpu_pinning: CpuPinningConfigAsc,
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
            cpu_pinning: Default::default(),
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
    /// Keep connections alive after sending a response
    pub keep_alive: bool,
    /// Does tracker run behind reverse proxy?
    ///
    /// MUST be set to false if not running behind reverse proxy.
    ///
    /// If set to true, make sure that reverse_proxy_ip_header_name and
    /// reverse_proxy_ip_header_format are set to match your reverse proxy
    /// setup.
    ///
    /// More info on what can go wrong when running behind reverse proxies:
    /// https://adam-p.ca/blog/2022/03/x-forwarded-for/
    pub runs_behind_reverse_proxy: bool,
    /// Name of header set by reverse proxy to indicate peer ip
    pub reverse_proxy_ip_header_name: String,
    /// How to extract peer IP from header field
    ///
    /// Options:
    /// - last_address: use the last address in the last instance of the
    ///   header. Works with typical multi-IP setups (e.g., "X-Forwarded-For")
    ///   as well as for single-IP setups (e.g., nginx "X-Real-IP")
    pub reverse_proxy_ip_header_format: ReverseProxyPeerIpHeaderFormat,
}

impl Default for NetworkConfig {
    fn default() -> Self {
        Self {
            address: SocketAddr::from(([0, 0, 0, 0], 3000)),
            enable_tls: false,
            tls_certificate_path: "".into(),
            tls_private_key_path: "".into(),
            only_ipv6: false,
            tcp_backlog: 1024,
            keep_alive: true,
            runs_behind_reverse_proxy: false,
            reverse_proxy_ip_header_name: "X-Forwarded-For".into(),
            reverse_proxy_ip_header_format: Default::default(),
        }
    }
}

#[derive(Clone, Debug, PartialEq, TomlConfig, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct ProtocolConfig {
    /// Maximum number of torrents to accept in scrape request
    pub max_scrape_torrents: usize,
    /// Maximum number of requested peers to accept in announce request
    pub max_peers: usize,
    /// Ask peers to announce this often (seconds)
    pub peer_announce_interval: usize,
}

impl Default for ProtocolConfig {
    fn default() -> Self {
        Self {
            max_scrape_torrents: 100,
            max_peers: 50,
            peer_announce_interval: 120,
        }
    }
}

#[derive(Clone, Debug, PartialEq, TomlConfig, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct CleaningConfig {
    /// Clean peers this often (seconds)
    pub torrent_cleaning_interval: u64,
    /// Clean connections this often (seconds)
    pub connection_cleaning_interval: u64,
    /// Remove peers that have not announced for this long (seconds)
    pub max_peer_age: u32,
    /// Remove connections that haven't seen valid requests for this long (seconds)
    pub max_connection_idle: u32,
}

impl Default for CleaningConfig {
    fn default() -> Self {
        Self {
            torrent_cleaning_interval: 30,
            connection_cleaning_interval: 60,
            max_peer_age: 1800,
            max_connection_idle: 180,
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
}

#[cfg(feature = "metrics")]
impl Default for MetricsConfig {
    fn default() -> Self {
        Self {
            run_prometheus_endpoint: false,
            prometheus_endpoint_address: SocketAddr::from(([0, 0, 0, 0], 9000)),
            torrent_count_update_interval: 10,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::Config;

    ::aquatic_toml_config::gen_serialize_deserialize_test!(Config);
}
