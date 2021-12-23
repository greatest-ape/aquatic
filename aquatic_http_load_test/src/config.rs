use std::net::SocketAddr;

use aquatic_cli_helpers::LogLevel;
use serde::{Deserialize};
use toml_config::TomlConfig;

/// aquatic_http_load_test configuration
#[derive(Clone, Debug, PartialEq, TomlConfig, Deserialize)]
#[serde(default)]
pub struct Config {
    pub server_address: SocketAddr,
    pub log_level: LogLevel,
    pub num_workers: usize,
    /// Maximum number of connections to keep open
    pub num_connections: usize,
    /// How often to check if num_connections connections are open, and
    /// open a new one otherwise. A value of 0 means that connections are
    /// opened as quickly as possible, which is useful when the tracker
    /// does not keep connections alive.
    pub connection_creation_interval_ms: u64,
    pub duration: usize,
    pub torrents: TorrentConfig,
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
            log_level: LogLevel::Error,
            num_workers: 1,
            num_connections: 128,
            connection_creation_interval_ms: 10,
            duration: 0,
            torrents: TorrentConfig::default(),
            #[cfg(feature = "cpu-pinning")]
            cpu_pinning: aquatic_common::cpu_pinning::CpuPinningConfig::default_for_load_test(),
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

#[cfg(test)]
mod tests {
    use super::Config;

    ::toml_config::gen_serialize_deserialize_test!(Config);
}
