use std::{net::SocketAddr, path::PathBuf};

use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(default)]
pub struct Config {
    pub server_address: SocketAddr,
    pub num_workers: u8,
    pub num_connections: usize,
    pub duration: usize,
    pub network: NetworkConfig,
    pub torrents: TorrentConfig,
}

impl aquatic_cli_helpers::Config for Config {}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(default)]
pub struct NetworkConfig {
    pub connection_creation_interval: usize,
    pub poll_timeout_microseconds: u64,
    pub poll_event_capacity: usize,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(default)]
pub struct TorrentConfig {
    pub number_of_torrents: usize,
    /// Pareto shape
    ///
    /// Fake peers choose torrents according to Pareto distribution.
    pub torrent_selection_pareto_shape: f64,
    /// Probability that a generated peer is a seeder
    pub peer_seeder_probability: f64,
    /// Probability that a generated request is a announce request, as part
    /// of sum of the various weight arguments.
    pub weight_announce: usize,
    /// Probability that a generated request is a scrape request, as part
    /// of sum of the various weight arguments.
    pub weight_scrape: usize,
}

impl Default for Config {
    fn default() -> Self {
        Self {
            server_address: "127.0.0.1:3000".parse().unwrap(),
            num_workers: 1,
            num_connections: 8,
            duration: 0,
            network: NetworkConfig::default(),
            torrents: TorrentConfig::default(),
        }
    }
}

impl Default for NetworkConfig {
    fn default() -> Self {
        Self {
            connection_creation_interval: 10,
            poll_timeout_microseconds: 197,
            poll_event_capacity: 64,
        }
    }
}

impl Default for TorrentConfig {
    fn default() -> Self {
        Self {
            number_of_torrents: 10_000,
            peer_seeder_probability: 0.25,
            torrent_selection_pareto_shape: 2.0,
            weight_announce: 5,
            weight_scrape: 0,
        }
    }
}
