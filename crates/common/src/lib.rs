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

/// Extract response peers
///
/// If there are more peers in map than `max_num_peers_to_take`, do a random
/// selection of peers from first and second halves of map in order to avoid
/// returning too homogeneous peers.
#[inline]
pub fn extract_response_peers<K, V, R, F>(
    rng: &mut impl Rng,
    peer_map: &IndexMap<K, V>,
    max_num_peers_to_take: usize,
    sender_peer_map_key: K,
    peer_conversion_function: F,
) -> Vec<R>
where
    K: Eq + ::std::hash::Hash,
    F: Fn(&K, &V) -> R,
{
    if peer_map.len() <= max_num_peers_to_take + 1 {
        // This branch: number of peers in map (minus sender peer) is less than
        // or equal to number of peers to take, so return all except sender
        // peer.
        let mut peers = Vec::with_capacity(peer_map.len());

        peers.extend(peer_map.iter().filter_map(|(k, v)| {
            (*k != sender_peer_map_key).then_some(peer_conversion_function(k, v))
        }));

        // Handle the case when sender peer is not in peer list. Typically,
        // this function will not be called when this is the case.
        if peers.len() > max_num_peers_to_take {
            peers.pop();
        }

        peers
    } else {
        // Note: if this branch is taken, the peer map contains at least two
        // more peers than max_num_peers_to_take

        let middle_index = peer_map.len() / 2;
        // Add one to take two extra peers in case sender peer is among
        // selected peers and will need to be filtered out
        let num_to_take_per_half = (max_num_peers_to_take / 2) + 1;

        let offset_half_one = {
            let from = 0;
            let to = usize::max(1, middle_index - num_to_take_per_half);

            rng.gen_range(from..to)
        };
        let offset_half_two = {
            let from = middle_index;
            let to = usize::max(middle_index + 1, peer_map.len() - num_to_take_per_half);

            rng.gen_range(from..to)
        };

        let end_half_one = offset_half_one + num_to_take_per_half;
        let end_half_two = offset_half_two + num_to_take_per_half;

        let mut peers = Vec::with_capacity(max_num_peers_to_take + 2);

        if let Some(slice) = peer_map.get_range(offset_half_one..end_half_one) {
            peers.extend(slice.iter().filter_map(|(k, v)| {
                (*k != sender_peer_map_key).then_some(peer_conversion_function(k, v))
            }));
        }
        if let Some(slice) = peer_map.get_range(offset_half_two..end_half_two) {
            peers.extend(slice.iter().filter_map(|(k, v)| {
                (*k != sender_peer_map_key).then_some(peer_conversion_function(k, v))
            }));
        }

        while peers.len() > max_num_peers_to_take {
            peers.pop();
        }

        peers
    }
}

#[cfg(test)]
mod tests {
    use ahash::HashSet;

    use rand::{rngs::SmallRng, SeedableRng};

    use super::*;

    #[test]
    fn test_extract_response_peers() {
        let mut rng = SmallRng::from_entropy();

        for num_peers_in_map in 0..50 {
            for max_num_peers_to_take in 0..50 {
                for sender_peer_map_key in 0..50 {
                    test_extract_response_peers_helper(
                        &mut rng,
                        num_peers_in_map,
                        max_num_peers_to_take,
                        sender_peer_map_key,
                    );
                }
            }
        }
    }

    fn test_extract_response_peers_helper(
        rng: &mut SmallRng,
        num_peers_in_map: usize,
        max_num_peers_to_take: usize,
        sender_peer_map_key: usize,
    ) {
        let peer_map = IndexMap::from_iter((0..num_peers_in_map).map(|i| (i, i)));

        let response_peers = extract_response_peers(
            rng,
            &peer_map,
            max_num_peers_to_take,
            sender_peer_map_key,
            |_, p| *p,
        );

        if num_peers_in_map > max_num_peers_to_take + 1 {
            assert_eq!(response_peers.len(), max_num_peers_to_take);
        } else {
            assert!(response_peers.len() <= max_num_peers_to_take);
        }

        assert!(!response_peers.contains(&sender_peer_map_key));

        assert_eq!(
            response_peers.len(),
            HashSet::from_iter(response_peers.iter().copied()).len()
        );
    }
}
