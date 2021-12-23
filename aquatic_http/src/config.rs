use std::{net::SocketAddr, path::PathBuf};

use aquatic_common::{access_list::AccessListConfig, privileges::PrivilegeConfig};
use serde::{Deserialize};
use toml_config::TomlConfig;

use aquatic_cli_helpers::LogLevel;

/// aquatic_http configuration
#[derive(Clone, Debug, PartialEq, TomlConfig, Deserialize)]
#[serde(default)]
pub struct Config {
    /// Socket workers receive requests from the socket, parse them and send
    /// them on to the request handler. They then recieve responses from the
    /// request handler, encode them and send them back over the socket.
    pub socket_workers: usize,
    /// Request workers receive a number of requests from socket workers,
    /// generate responses and send them back to the socket workers.
    pub request_workers: usize,
    pub log_level: LogLevel,
    pub network: NetworkConfig,
    pub protocol: ProtocolConfig,
    pub cleaning: CleaningConfig,
    pub privileges: PrivilegeConfig,
    pub access_list: AccessListConfig,
    #[cfg(feature = "cpu-pinning")]
    pub cpu_pinning: aquatic_common::cpu_pinning::CpuPinningConfig,
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
    pub tls_certificate_path: PathBuf,
    pub tls_private_key_path: PathBuf,
    /// Only allow access over IPv6
    pub ipv6_only: bool,
    /// Keep connections alive
    pub keep_alive: bool,
}

#[derive(Clone, Debug, PartialEq, TomlConfig, Deserialize)]
#[serde(default)]
pub struct ProtocolConfig {
    /// Maximum number of torrents to accept in scrape request
    pub max_scrape_torrents: usize,
    /// Maximum number of requested peers to accept in announce request
    pub max_peers: usize,
    /// Ask peers to announce this often (seconds)
    pub peer_announce_interval: usize,
}

#[derive(Clone, Debug, PartialEq, TomlConfig, Deserialize)]
#[serde(default)]
pub struct CleaningConfig {
    /// Clean peers this often (seconds)
    pub torrent_cleaning_interval: u64,
    /// Remove peers that have not announced for this long (seconds)
    pub max_peer_age: u64,
}

impl Default for Config {
    fn default() -> Self {
        Self {
            socket_workers: 1,
            request_workers: 1,
            log_level: LogLevel::default(),
            network: NetworkConfig::default(),
            protocol: ProtocolConfig::default(),
            cleaning: CleaningConfig::default(),
            privileges: PrivilegeConfig::default(),
            access_list: AccessListConfig::default(),
            #[cfg(feature = "cpu-pinning")]
            cpu_pinning: Default::default(),
        }
    }
}

impl Default for NetworkConfig {
    fn default() -> Self {
        Self {
            address: SocketAddr::from(([0, 0, 0, 0], 3000)),
            tls_certificate_path: "".into(),
            tls_private_key_path: "".into(),
            ipv6_only: false,
            keep_alive: true,
        }
    }
}

impl Default for ProtocolConfig {
    fn default() -> Self {
        Self {
            max_scrape_torrents: 255, // FIXME: what value is reasonable?
            max_peers: 50,
            peer_announce_interval: 120,
        }
    }
}

impl Default for CleaningConfig {
    fn default() -> Self {
        Self {
            torrent_cleaning_interval: 30,
            max_peer_age: 1800,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::Config;

    ::toml_config::gen_serialize_deserialize_test!(Config);
}
