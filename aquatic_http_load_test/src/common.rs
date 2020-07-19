use std::net::SocketAddr;
use std::sync::{Arc, atomic::AtomicUsize};

use serde::{Serialize, Deserialize};
use rand_distr::Pareto;

pub use aquatic_http_protocol::common::*;
pub use aquatic_http_protocol::response::*;
pub use aquatic_http_protocol::request::*;


#[derive(Debug, PartialEq, Eq, Hash, Clone, Copy)]
pub struct ThreadId(pub u8);


#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(default)]
pub struct Config {
    /// Server address
    pub server_address: SocketAddr,
    /// Number of sockets and socket worker threads
    /// 
    /// Sockets will bind to one port each, and with
    /// multiple_client_ips = true, additionally to one IP each.
    pub num_socket_workers: u8,
    /// Number of workers generating requests from responses, as well as
    /// requests not connected to previous ones.
    pub num_request_workers: usize,
    /// Run duration (quit and generate report after this many seconds)
    pub duration: usize,
    pub network: NetworkConfig,
    pub handler: HandlerConfig,
}


#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(default)]
pub struct NetworkConfig {
    /// True means bind to one localhost IP per socket. On macOS, this by
    /// default causes all server responses to go to one socket worker.
    /// Default option ("true") can cause issues on macOS.
    /// 
    /// The point of multiple IPs is to possibly cause a better distribution
    /// of requests to servers with SO_REUSEPORT option.
    pub multiple_client_ips: bool,
    /// Use Ipv6 only
    pub ipv6_client: bool,
    /// Number of first client port
    pub first_port: u16,
    /// Socket worker poll timeout in microseconds
    pub poll_timeout: u64,
    /// Socket worker polling event number
    pub poll_event_capacity: usize,
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
    pub recv_buffer: usize,
}


#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(default)]
pub struct HandlerConfig {
    /// Number of torrents to simulate
    pub number_of_torrents: usize,
    /// Maximum number of torrents to ask about in scrape requests
    pub scrape_max_torrents: usize,
    /// Handler: max number of responses to collect for before processing
    pub max_responses_per_iter: usize,
    /// Probability that a generated request is a announce request, as part
    /// of sum of the various weight arguments.
    pub weight_announce: usize,
    /// Probability that a generated request is a scrape request, as part
    /// of sum of the various weight arguments.
    pub weight_scrape: usize,
    /// Handler: max microseconds to wait for single response from channel
    pub channel_timeout: u64,
    /// Pareto shape
    /// 
    /// Fake peers choose torrents according to Pareto distribution.
    pub torrent_selection_pareto_shape: f64,
    /// Probability that a generated peer is a seeder
    pub peer_seeder_probability: f64,
    /// Part of additional request creation calculation, meaning requests
    /// which are not dependent on previous responses from server. Higher
    /// means more.
    pub additional_request_factor: f64,
}


impl Default for Config {
    fn default() -> Self {
        Self {
            server_address: "127.0.0.1:3000".parse().unwrap(),
            num_socket_workers: 1,
            num_request_workers: 1,
            duration: 0,
            network: NetworkConfig::default(),
            handler: HandlerConfig::default(),
        }
    }
}

impl Default for NetworkConfig {
    fn default() -> Self {
        Self {
            multiple_client_ips: true,
            ipv6_client: false,
            first_port: 45_000,
            poll_timeout: 276,
            poll_event_capacity: 2_877,
            recv_buffer: 6_000_000,
        }
    }
}


impl Default for HandlerConfig {
    fn default() -> Self {
        Self {
            number_of_torrents: 10_000,
            peer_seeder_probability: 0.25,
            scrape_max_torrents: 50,
            weight_announce: 5,
            weight_scrape: 1,
            additional_request_factor: 0.4,
            max_responses_per_iter: 10_000,
            channel_timeout: 200,
            torrent_selection_pareto_shape: 2.0,
        }
    }
}


#[derive(PartialEq, Eq, Clone)]
pub struct TorrentPeer {
    pub info_hash: InfoHash,
    pub scrape_hash_indeces: Vec<usize>,
    pub peer_id: PeerId,
    pub port: u16,
}


#[derive(Default)]
pub struct Statistics {
    pub requests: AtomicUsize,
    pub response_peers: AtomicUsize,
    pub responses_announce: AtomicUsize,
    pub responses_scrape: AtomicUsize,
    pub responses_failure: AtomicUsize,
}


#[derive(Clone)]
pub struct LoadTestState {
    pub info_hashes: Arc<Vec<InfoHash>>,
    pub statistics: Arc<Statistics>,
    pub pareto: Arc<Pareto<f64>>,
}


#[derive(PartialEq, Eq, Clone, Copy)]
pub enum RequestType {
    Announce,
    Scrape
}


#[derive(Default)]
pub struct SocketWorkerLocalStatistics {
    pub requests: usize,
    pub response_peers: usize,
    pub responses_announce: usize,
    pub responses_scrape: usize,
    pub responses_failure: usize,
}