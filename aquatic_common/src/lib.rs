use std::net::{Ipv4Addr, SocketAddr, SocketAddrV4, SocketAddrV6};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};

use ahash::RandomState;
use rand::Rng;

pub mod access_list;
pub mod cpu_pinning;
pub mod privileges;
#[cfg(feature = "rustls-config")]
pub mod rustls_config;

/// Amortized IndexMap using AHash hasher
pub type AmortizedIndexMap<K, V> = indexmap_amortized::IndexMap<K, V, RandomState>;

/// Peer or connection valid until this instant
///
/// Used instead of "last seen" or similar to hopefully prevent arithmetic
/// overflow when cleaning.
#[derive(Debug, Clone, Copy)]
pub struct ValidUntil(pub Instant);

impl ValidUntil {
    #[inline]
    pub fn new(offset_seconds: u64) -> Self {
        Self(Instant::now() + Duration::from_secs(offset_seconds))
    }
    pub fn new_with_now(now: Instant, offset_seconds: u64) -> Self {
        Self(now + Duration::from_secs(offset_seconds))
    }
}

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

            if !already_triggered {
                if unsafe { libc::raise(15) } == -1 {
                    panic!(
                        "Could not raise SIGTERM: {:#}",
                        ::std::io::Error::last_os_error()
                    )
                }
            }
        }
    }
}

/// Extract response peers
///
/// If there are more peers in map than `max_num_peers_to_take`, do a
/// half-random selection of peers from first and second halves of map,
/// in order to avoid returning too homogeneous peers.
///
/// Might return one less peer than wanted since sender is filtered out.
#[inline]
pub fn extract_response_peers<K, V, R, F>(
    rng: &mut impl Rng,
    peer_map: &AmortizedIndexMap<K, V>,
    max_num_peers_to_take: usize,
    sender_peer_map_key: K,
    peer_conversion_function: F,
) -> Vec<R>
where
    K: Eq + ::std::hash::Hash,
    F: Fn(&V) -> R,
{
    let peer_map_len = peer_map.len();

    if peer_map_len <= max_num_peers_to_take + 1 {
        let mut peers = Vec::with_capacity(peer_map_len);

        peers.extend(peer_map.iter().filter_map(|(k, v)| {
            if *k == sender_peer_map_key {
                None
            } else {
                Some(peer_conversion_function(v))
            }
        }));

        peers
    } else {
        let half_num_to_take = max_num_peers_to_take / 2;
        let half_peer_map_len = peer_map_len / 2;

        let offset_first_half =
            rng.gen_range(0..(half_peer_map_len + (peer_map_len % 2)) - half_num_to_take);
        let offset_second_half = rng.gen_range(half_peer_map_len..peer_map_len - half_num_to_take);

        let end_first_half = offset_first_half + half_num_to_take;
        let end_second_half = offset_second_half + half_num_to_take + (max_num_peers_to_take % 2);

        let mut peers: Vec<R> = Vec::with_capacity(max_num_peers_to_take);

        for i in offset_first_half..end_first_half {
            if let Some((k, peer)) = peer_map.get_index(i) {
                if *k != sender_peer_map_key {
                    peers.push(peer_conversion_function(peer))
                }
            }
        }
        for i in offset_second_half..end_second_half {
            if let Some((k, peer)) = peer_map.get_index(i) {
                if *k != sender_peer_map_key {
                    peers.push(peer_conversion_function(peer))
                }
            }
        }

        peers
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
