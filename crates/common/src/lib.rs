use std::net::{Ipv4Addr, SocketAddr, SocketAddrV4, SocketAddrV6};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::Instant;

use ahash::RandomState;
use rand::Rng;

pub mod access_list;
pub mod cli;
pub mod cpu_pinning;
pub mod privileges;
#[cfg(feature = "rustls")]
pub mod rustls_config;

/// IndexMap using AHash hasher
pub type IndexMap<K, V> = indexmap::IndexMap<K, V, RandomState>;

/// Peer, connection or similar valid until this instant
#[derive(Debug, Clone, Copy)]
pub struct ValidUntil(SecondsSinceServerStart);

impl ValidUntil {
    #[inline]
    pub fn new(start_instant: ServerStartInstant, offset_seconds: u32) -> Self {
        Self(SecondsSinceServerStart(
            start_instant.seconds_elapsed().0 + offset_seconds,
        ))
    }
    pub fn new_with_now(now: SecondsSinceServerStart, offset_seconds: u32) -> Self {
        Self(SecondsSinceServerStart(now.0 + offset_seconds))
    }
    pub fn valid(&self, now: SecondsSinceServerStart) -> bool {
        self.0 .0 > now.0
    }
}

#[derive(Debug, Clone, Copy)]
pub struct ServerStartInstant(Instant);

impl ServerStartInstant {
    #[allow(clippy::new_without_default)] // I prefer ::new here
    pub fn new() -> Self {
        Self(Instant::now())
    }
    pub fn seconds_elapsed(&self) -> SecondsSinceServerStart {
        SecondsSinceServerStart(
            self.0
                .elapsed()
                .as_secs()
                .try_into()
                .expect("server ran for more seconds than what fits in a u32"),
        )
    }
}

#[derive(Debug, Clone, Copy)]
pub struct SecondsSinceServerStart(u32);

pub struct PanicSentinelWatcher(Arc<AtomicBool>);

impl PanicSentinelWatcher {
    pub fn create_with_sentinel() -> (Self, PanicSentinel) {
        let triggered = Arc::new(AtomicBool::new(false));
        let sentinel = PanicSentinel(triggered.clone());

        (Self(triggered), sentinel)
    }

    pub fn panic_was_triggered(&self) -> bool {
        self.0.load(Ordering::SeqCst)
    }
}

/// Raises SIGTERM when dropped
///
/// Pass to threads to have panics in them cause whole program to exit.
#[derive(Clone)]
pub struct PanicSentinel(Arc<AtomicBool>);

impl Drop for PanicSentinel {
    fn drop(&mut self) {
        if ::std::thread::panicking() {
            let already_triggered = self.0.fetch_or(true, Ordering::SeqCst);

            if !already_triggered && unsafe { libc::raise(15) } == -1 {
                panic!(
                    "Could not raise SIGTERM: {:#}",
                    ::std::io::Error::last_os_error()
                )
            }
        }
    }
}

/// SocketAddr that is not an IPv6-mapped IPv4 address
#[derive(Clone, Copy, Debug, Hash, PartialEq, Eq)]
pub struct CanonicalSocketAddr(SocketAddr);

impl CanonicalSocketAddr {
    pub fn new(addr: SocketAddr) -> Self {
        match addr {
            addr @ SocketAddr::V4(_) => Self(addr),
            SocketAddr::V6(addr) => {
                match addr.ip().octets() {
                    // Convert IPv4-mapped address (available in std but nightly-only)
                    [0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0xff, 0xff, a, b, c, d] => Self(SocketAddr::V4(
                        SocketAddrV4::new(Ipv4Addr::new(a, b, c, d), addr.port()),
                    )),
                    _ => Self(addr.into()),
                }
            }
        }
    }

    pub fn get_ipv6_mapped(self) -> SocketAddr {
        match self.0 {
            SocketAddr::V4(addr) => {
                let ip = addr.ip().to_ipv6_mapped();

                SocketAddr::V6(SocketAddrV6::new(ip, addr.port(), 0, 0))
            }
            addr => addr,
        }
    }

    pub fn get(self) -> SocketAddr {
        self.0
    }

    pub fn get_ipv4(self) -> Option<SocketAddr> {
        match self.0 {
            addr @ SocketAddr::V4(_) => Some(addr),
            _ => None,
        }
    }

    pub fn is_ipv4(&self) -> bool {
        self.0.is_ipv4()
    }
}
