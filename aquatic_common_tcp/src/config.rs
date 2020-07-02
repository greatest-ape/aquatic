use std::net::SocketAddr;

use serde::{Serialize, Deserialize};


#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum LogLevel {
    Off,
    Error,
    Warn,
    Info,
    Debug,
    Trace
}


impl Default for LogLevel {
    fn default() -> Self {
        Self::Error
    }
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
pub struct SocketConfig {
    /// Bind to this address
    pub address: SocketAddr,
    pub ipv6_only: bool,
}


#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(default)]
pub struct TlsConfig {
    pub use_tls: bool,
    pub tls_pkcs12_path: String,
    pub tls_pkcs12_password: String,
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


impl Default for HandlerConfig {
    fn default() -> Self {
        Self {
            max_requests_per_iter: 10000,
            channel_recv_timeout_microseconds: 200,
        }
    }
}


impl Default for SocketConfig {
    fn default() -> Self {
        Self {
            address: SocketAddr::from(([0, 0, 0, 0], 3000)),
            ipv6_only: false,
        }
    }
}


impl Default for TlsConfig {
    fn default() -> Self {
        Self {
            use_tls: false,
            tls_pkcs12_path: "".into(),
            tls_pkcs12_password: "".into(),
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