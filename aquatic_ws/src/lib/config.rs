use std::net::SocketAddr;

use serde::{Serialize, Deserialize};


#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(default)]
pub struct Config {
    /// Socket workers receive requests from the socket, parse them and send
    /// them on to the request handler. They then recieve responses from the
    /// request handler, encode them and send them back over the socket.
    pub socket_workers: usize,
    pub network: NetworkConfig,
    pub handlers: HandlerConfig,
    pub cleaning: CleaningConfig,
    // pub privileges: PrivilegeConfig,
}


#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(default)]
pub struct NetworkConfig {
    /// Bind to this address
    pub address: SocketAddr,
    pub use_tls: bool,
    pub tls_pkcs12_path: String,
    pub tls_pkcs12_password: String,
    /// Maximum number of torrents to accept in scrape request
    pub max_scrape_torrents: usize, // FIXME: should this really be in NetworkConfig?
    /// Maximum number of offers to accept in announce request
    pub max_offers: usize, // FIXME: should this really be in NetworkConfig?
    /// Ask peers to announce this often (seconds)
    pub peer_announce_interval: usize, // FIXME: should this really be in NetworkConfig?
    pub poll_event_capacity: usize,
    pub poll_timeout_milliseconds: u64,
}


#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(default)]
pub struct HandlerConfig {
    /// Maximum number of requests to receive from channel before locking
    /// mutex and starting work
    pub max_requests_per_iter: usize,
    pub channel_recv_timeout_microseconds: u64,
}


#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(default)]
pub struct CleaningConfig {
    /// Clean peers this often (seconds)
    pub interval: u64,
    /// Remove peers that haven't announced for this long (seconds)
    pub max_peer_age: u64,
    /// Remove connections that are older than this (seconds)
    pub max_connection_age: u64,
}


// FIXME: implement
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(default)]
pub struct PrivilegeConfig {
    /// Chroot and switch user after binding to sockets
    pub drop_privileges: bool,
    /// Chroot to this path
    pub chroot_path: String,
    /// User to switch to after chrooting
    pub user: String,
}


impl Default for Config {
    fn default() -> Self {
        Self {
            socket_workers: 1,
            network: NetworkConfig::default(),
            handlers: HandlerConfig::default(),
            cleaning: CleaningConfig::default(),
            // privileges: PrivilegeConfig::default(),
        }
    }
}


impl Default for NetworkConfig {
    fn default() -> Self {
        Self {
            address: SocketAddr::from(([0, 0, 0, 0], 3000)),
            use_tls: false,
            tls_pkcs12_path: "".into(),
            tls_pkcs12_password: "".into(),
            max_scrape_torrents: 255, // FIXME: what value is reasonable?
            max_offers: 10,
            peer_announce_interval: 120,
            poll_event_capacity: 4096,
            poll_timeout_milliseconds: 50,
        }
    }
}


impl Default for HandlerConfig {
    fn default() -> Self {
        Self {
            max_requests_per_iter: 10000,
            channel_recv_timeout_microseconds: 200,
        }
    }
}


impl Default for CleaningConfig {
    fn default() -> Self {
        Self {
            interval: 30,
            max_peer_age: 180,
            max_connection_age: 180,
        }
    }
}


impl Default for PrivilegeConfig {
    fn default() -> Self {
        Self {
            drop_privileges: false,
            chroot_path: ".".to_string(),
            user: "nobody".to_string(),
        }
    }
}