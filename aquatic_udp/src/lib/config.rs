use std::{net::SocketAddr, time::Duration};

use aquatic_common::{access_list::AccessListConfig, privileges::PrivilegeConfig};
use serde::{Deserialize, Serialize};

use aquatic_cli_helpers::LogLevel;

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(default)]
pub struct Config {
    /// Socket workers receive requests from the socket, parse them and send
    /// them on to the request workers. They then recieve responses from the
    /// request workers, encode them and send them back over the socket.
    pub socket_workers: usize,
    /// Request workers receive a number of requests from socket workers,
    /// generate responses and send them back to the socket workers.
    pub request_workers: usize,
    pub log_level: LogLevel,
    pub network: NetworkConfig,
    pub protocol: ProtocolConfig,
    pub handlers: HandlerConfig,
    pub statistics: StatisticsConfig,
    pub cleaning: CleaningConfig,
    pub privileges: PrivilegeConfig,
    pub access_list: AccessListConfig,
    #[cfg(feature = "cpu-pinning")]
    pub cpu_pinning: aquatic_common::cpu_pinning::CpuPinningConfig,
}

impl Default for Config {
    fn default() -> Self {
        Self {
            socket_workers: 1,
            request_workers: 1,
            log_level: LogLevel::Error,
            network: NetworkConfig::default(),
            protocol: ProtocolConfig::default(),
            handlers: HandlerConfig::default(),
            statistics: StatisticsConfig::default(),
            cleaning: CleaningConfig::default(),
            privileges: PrivilegeConfig::default(),
            access_list: AccessListConfig::default(),
            #[cfg(feature = "cpu-pinning")]
            cpu_pinning: Default::default(),
        }
    }
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
    pub only_ipv6: bool,
    /// Size of socket recv buffer. Use 0 for OS default.
    ///
    /// This setting can have a big impact on dropped packages. It might
    /// require changing system defaults. Some examples of commands to set
    /// recommended values for different operating systems:
    ///
    /// macOS:
    /// $ sudo sysctl net.inet.udp.recvspace=6000000
    /// $ sudo sysctl net.inet.udp.maxdgram=500000 # Not necessary, but recommended
    /// $ sudo sysctl kern.ipc.maxsockbuf=8388608 # Not necessary, but recommended
    ///
    /// Linux:
    /// $ sudo sysctl -w net.core.rmem_max=104857600
    /// $ sudo sysctl -w net.core.rmem_default=104857600
    pub socket_recv_buffer_size: usize,
    pub poll_event_capacity: usize,
    #[serde(with = "serde_duration_milliseconds")]
    pub poll_timeout_ms: Duration,
}

impl Default for NetworkConfig {
    fn default() -> Self {
        Self {
            address: SocketAddr::from(([0, 0, 0, 0], 3000)),
            only_ipv6: false,
            socket_recv_buffer_size: 4096 * 128,
            poll_event_capacity: 4096,
            poll_timeout_ms: Duration::from_millis(50),
        }
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(default)]
pub struct ProtocolConfig {
    /// Maximum number of torrents to accept in scrape request
    pub max_scrape_torrents: u8,
    /// Maximum number of peers to return in announce response
    pub max_response_peers: usize,
    /// Ask peers to announce this often (seconds)
    ///
    /// This field uses i32 instead of Duration because so that it can be
    /// passed verbatim to peers.
    pub peer_announce_interval: i32,
}

impl Default for ProtocolConfig {
    fn default() -> Self {
        Self {
            max_scrape_torrents: 255,
            max_response_peers: 255,
            peer_announce_interval: 60 * 15,
        }
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(default)]
pub struct HandlerConfig {
    #[serde(with = "serde_duration_milliseconds")]
    pub channel_recv_timeout_ms: Duration,
}

impl Default for HandlerConfig {
    fn default() -> Self {
        Self {
            channel_recv_timeout_ms: Duration::from_millis(100),
        }
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(default)]
pub struct StatisticsConfig {
    /// Print statistics this often (seconds). Don't print when set to zero.
    #[serde(with = "serde_duration_seconds")]
    pub interval: Duration,
}

impl Default for StatisticsConfig {
    fn default() -> Self {
        Self {
            interval: Duration::default(),
        }
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(default)]
pub struct CleaningConfig {
    /// Clean connections this often (seconds)
    #[serde(with = "serde_duration_seconds")]
    pub connection_cleaning_interval: Duration,
    /// Clean torrents this often (seconds)
    #[serde(with = "serde_duration_seconds")]
    pub torrent_cleaning_interval: Duration,
    /// Clean pending scrape responses this often (seconds)
    ///
    /// In regular operation, there should be no pending scrape responses
    /// lingering for a long time. However, the cleaning also returns unused
    /// allocated memory to the OS, so the interval can be configured here.
    #[serde(with = "serde_duration_seconds")]
    pub pending_scrape_cleaning_interval: Duration,
    /// Remove connections that are older than this (seconds)
    #[serde(with = "serde_duration_seconds")]
    pub max_connection_age: Duration,
    /// Remove peers that haven't announced for this long (seconds)
    #[serde(with = "serde_duration_seconds")]
    pub max_peer_age: Duration,
    /// Remove pending scrape responses that haven't been returned from request
    /// workers for this long (seconds)
    #[serde(with = "serde_duration_seconds")]
    pub max_pending_scrape_age: Duration,
}

impl Default for CleaningConfig {
    fn default() -> Self {
        Self {
            connection_cleaning_interval: Duration::from_secs(60),
            torrent_cleaning_interval: Duration::from_secs(60 * 2),
            pending_scrape_cleaning_interval: Duration::from_secs(60 * 10),
            max_connection_age: Duration::from_secs(60 * 5),
            max_peer_age: Duration::from_secs(60 * 20),
            max_pending_scrape_age: Duration::from_secs(60),
        }
    }
}

pub mod serde_duration_seconds {
    use std::time::Duration;

    use serde::{de::Visitor, Deserializer, Serializer};

    struct DurationVisitor;

    impl<'de> Visitor<'de> for DurationVisitor {
        type Value = Duration;

        fn expecting(&self, formatter: &mut ::std::fmt::Formatter) -> ::std::fmt::Result {
            write!(formatter, "a number of seconds")
        }

        fn visit_i64<E>(self, seconds: i64) -> Result<Self::Value, E>
        where
            E: serde::de::Error,
        {
            Ok(Duration::from_secs(seconds as u64))
        }
    }

    pub fn serialize<S: Serializer>(duration: &Duration, serializer: S) -> Result<S::Ok, S::Error> {
        serializer.serialize_u64(duration.as_secs())
    }
    pub fn deserialize<'de, D: Deserializer<'de>>(deserializer: D) -> Result<Duration, D::Error> {
        deserializer.deserialize_u64(DurationVisitor)
    }
}

pub mod serde_duration_milliseconds {
    use std::{convert::TryInto, time::Duration};

    use serde::{de::Visitor, Deserializer, Serializer};

    struct DurationVisitor;

    impl<'de> Visitor<'de> for DurationVisitor {
        type Value = Duration;

        fn expecting(&self, formatter: &mut ::std::fmt::Formatter) -> ::std::fmt::Result {
            write!(formatter, "a number of milliseconds")
        }

        fn visit_i64<E>(self, millis: i64) -> Result<Self::Value, E>
        where
            E: serde::de::Error,
        {
            Ok(Duration::from_millis(millis as u64))
        }
    }

    pub fn serialize<S: Serializer>(duration: &Duration, serializer: S) -> Result<S::Ok, S::Error> {
        serializer.serialize_u64(duration.as_millis().try_into().unwrap())
    }
    pub fn deserialize<'de, D: Deserializer<'de>>(deserializer: D) -> Result<Duration, D::Error> {
        deserializer.deserialize_u64(DurationVisitor)
    }
}
