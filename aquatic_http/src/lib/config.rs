use std::net::SocketAddr;

use serde::{Serialize, Deserialize};

pub use aquatic_common_tcp::config::*;


#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(default)]
pub struct Config {
    /// Socket workers receive requests from the socket, parse them and send
    /// them on to the request handler. They then recieve responses from the
    /// request handler, encode them and send them back over the socket.
    pub socket_workers: usize,
    pub log_level: LogLevel,
    pub network: NetworkConfig,
    pub protocol: ProtocolConfig,
    pub handlers: HandlerConfig,
    pub cleaning: CleaningConfig,
    pub privileges: PrivilegeConfig,
}


#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(default)]
pub struct NetworkConfig {
    /// Bind to this address
    pub address: SocketAddr,
    pub ipv6_only: bool,
    #[serde(flatten)]
    pub tls: TlsConfig,
    pub poll_event_capacity: usize,
    pub poll_timeout_milliseconds: u64,
}



#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(default)]
pub struct ProtocolConfig {
    /// Maximum number of torrents to accept in scrape request
    pub max_scrape_torrents: usize,
    /// Maximum number of requested peers to accept in announce request
    pub max_peers: usize,
    /// Ask peers to announce this often (seconds)
    pub peer_announce_interval: usize,
}



impl Default for Config {
    fn default() -> Self {
        Self {
            socket_workers: 1,
            log_level: LogLevel::default(),
            network: NetworkConfig::default(),
            protocol: ProtocolConfig::default(),
            handlers: HandlerConfig::default(),
            cleaning: CleaningConfig::default(),
            privileges: PrivilegeConfig::default(),
        }
    }
}


impl Default for NetworkConfig {
    fn default() -> Self {
        Self {
            address: SocketAddr::from(([0, 0, 0, 0], 3000)),
            ipv6_only: false,
            tls: TlsConfig::default(),
            poll_event_capacity: 4096,
            poll_timeout_milliseconds: 50,
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