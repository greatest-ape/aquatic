use std::net::SocketAddr;

use serde::{Serialize, Deserialize};


#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Config {
    /// Spawn this number of threads for workers
    pub socket_workers: usize,
    pub response_workers: usize,
    pub network: NetworkConfig,
    pub statistics: StatisticsConfig,
    pub cleaning: CleaningConfig,
}


#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct NetworkConfig {
    /// Bind to this address
    pub address: SocketAddr,
    /// Maximum number of torrents to accept in scrape request
    pub max_scrape_torrents: u8,
    /// Maximum number of peers to return in announce response
    pub max_response_peers: usize,
    /// Ask peers to announce this often (seconds)
    pub peer_announce_interval: i32,
    /// Setting on socket. When value is zero, don't set (use OS default)
    pub recv_buffer_size: usize,
    pub poll_event_capacity: usize,
}


#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct StatisticsConfig {
    /// Print statistics this often (seconds). Don't print when set to zero.
    pub interval: u64,
}


#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct CleaningConfig {
    /// Clean torrents and connections this often (seconds)
    pub interval: u64,
    /// Remove peers that haven't announced for this long (seconds)
    pub max_peer_age: u64,
    /// Remove connections that are older than this (seconds)
    pub max_connection_age: u64,
}


impl Default for Config {
    fn default() -> Self {
        Self {
            socket_workers: 1,
            response_workers: 1,
            network: NetworkConfig::default(),
            statistics: StatisticsConfig::default(),
            cleaning: CleaningConfig::default(),
        }
    }
}


impl Default for NetworkConfig {
    fn default() -> Self {
        Self {
            address: SocketAddr::from(([127, 0, 0, 1], 3000)),
            max_scrape_torrents: 255,
            max_response_peers: 255,
            peer_announce_interval: 60 * 15,
            poll_event_capacity: 4096,
            recv_buffer_size: 4096 * 128,
        }
    }
}


impl Default for StatisticsConfig {
    fn default() -> Self {
        Self {
            interval: 5,
        }
    }
}


impl Default for CleaningConfig {
    fn default() -> Self {
        Self {
            interval: 30,
            max_peer_age: 60 * 20,
            max_connection_age: 60 * 5,
        }
    }
}
