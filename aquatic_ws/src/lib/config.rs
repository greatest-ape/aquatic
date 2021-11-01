use std::net::SocketAddr;
use std::path::PathBuf;

use aquatic_common::cpu_pinning::CpuPinningConfig;
use aquatic_common::{access_list::AccessListConfig, privileges::PrivilegeConfig};
use serde::{Deserialize, Serialize};

use aquatic_cli_helpers::LogLevel;

#[derive(Clone, Debug, Serialize, Deserialize)]
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
    pub cpu_pinning: CpuPinningConfig,
}

impl aquatic_cli_helpers::Config for Config {
    fn get_log_level(&self) -> Option<LogLevel> {
        Some(self.log_level)
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(default)]
pub struct NetworkConfig {
    /// Bind to this address
    pub address: SocketAddr,
    pub ipv6_only: bool,
    pub tls_certificate_path: PathBuf,
    pub tls_private_key_path: PathBuf,
    pub websocket_max_message_size: usize,
    pub websocket_max_frame_size: usize,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(default)]
pub struct ProtocolConfig {
    /// Maximum number of torrents to accept in scrape request
    pub max_scrape_torrents: usize,
    /// Maximum number of offers to accept in announce request
    pub max_offers: usize,
    /// Ask peers to announce this often (seconds)
    pub peer_announce_interval: usize,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(default)]
pub struct CleaningConfig {
    /// Clean peers this often (seconds)
    pub interval: u64,
    /// Remove peers that haven't announced for this long (seconds)
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
            cpu_pinning: Default::default(),
        }
    }
}

impl Default for NetworkConfig {
    fn default() -> Self {
        Self {
            address: SocketAddr::from(([0, 0, 0, 0], 3000)),
            ipv6_only: false,
            tls_certificate_path: "".into(),
            tls_private_key_path: "".into(),
            websocket_max_message_size: 64 * 1024,
            websocket_max_frame_size: 16 * 1024,
        }
    }
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

impl Default for CleaningConfig {
    fn default() -> Self {
        Self {
            interval: 30,
            max_peer_age: 1800,
        }
    }
}
