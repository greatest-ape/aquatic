use std::net::SocketAddr;
use std::path::PathBuf;

use aquatic_common::cpu_pinning::asc::CpuPinningConfigAsc;
use aquatic_common::{access_list::AccessListConfig, privileges::PrivilegeConfig};
use serde::Deserialize;

use aquatic_common::cli::LogLevel;
use aquatic_toml_config::TomlConfig;

/// aquatic_ws configuration
#[derive(Clone, Debug, PartialEq, TomlConfig, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct Config {
    /// Socket workers receive requests from the socket, parse them and send
    /// them on to the swarm workers. They then receive responses from the
    /// swarm workers, encode them and send them back over the socket.
    pub socket_workers: usize,
    /// Swarm workers receive a number of requests from socket workers,
    /// generate responses and send them back to the socket workers.
    pub swarm_workers: usize,
    pub log_level: LogLevel,
    pub network: NetworkConfig,
    pub protocol: ProtocolConfig,
    pub cleaning: CleaningConfig,
    pub privileges: PrivilegeConfig,
    pub access_list: AccessListConfig,
    pub cpu_pinning: CpuPinningConfigAsc,
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

    /// Path to TLS certificate (DER-encoded X.509)
    pub tls_certificate_path: PathBuf,
    /// Path to TLS private key (DER-encoded ASN.1 in PKCS#8 or PKCS#1 format)
    pub tls_private_key_path: PathBuf,

    pub websocket_max_message_size: usize,
    pub websocket_max_frame_size: usize,
}

impl Default for NetworkConfig {
    fn default() -> Self {
        Self {
            address: SocketAddr::from(([0, 0, 0, 0], 3000)),
            only_ipv6: false,
            tcp_backlog: 1024,

            tls_certificate_path: "".into(),
            tls_private_key_path: "".into(),

            websocket_max_message_size: 64 * 1024,
            websocket_max_frame_size: 16 * 1024,
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
    pub max_peer_age: u64,
    // Clean connections this often (seconds)
    pub connection_cleaning_interval: u64,
    /// Close connections if no responses have been sent to them for this long (seconds)
    pub max_connection_idle: u64,
}

impl Default for CleaningConfig {
    fn default() -> Self {
        Self {
            torrent_cleaning_interval: 30,
            max_peer_age: 1800,
            max_connection_idle: 60 * 5,
            connection_cleaning_interval: 30,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::Config;

    ::aquatic_toml_config::gen_serialize_deserialize_test!(Config);
}
