use std::time::{Duration, Instant};

use indexmap::IndexMap;
use rand::Rng;


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
}


/// Extract response peers
/// 
/// If there are more peers in map than `max_num_peers_to_take`, do a
/// half-random selection of peers from first and second halves of map,
/// in order to avoid returning too homogeneous peers.
/// 
/// Don't care if we send back announcing peer.
#[inline]
pub fn extract_response_peers<K, V, R, F>(
    rng: &mut impl Rng,
    peer_map: &IndexMap<K, V>,
    max_num_peers_to_take: usize,
    peer_conversion_function: F
) -> Vec<R>
    where
        K: Eq + ::std::hash::Hash,
        F: Fn(&V) -> R
{
    let peer_map_len = peer_map.len();

    if peer_map_len <= max_num_peers_to_take {
        peer_map.values()
            .map(peer_conversion_function)
            .collect()
    } else {
        let half_num_to_take = max_num_peers_to_take / 2;
        let half_peer_map_len = peer_map_len / 2;

        let offset_first_half = rng.gen_range(
            0,
            (half_peer_map_len + (peer_map_len % 2)) - half_num_to_take
        );
        let offset_second_half = rng.gen_range(
            half_peer_map_len,
            peer_map_len - half_num_to_take
        );

        let end_first_half = offset_first_half + half_num_to_take;
        let end_second_half = offset_second_half + half_num_to_take + (max_num_peers_to_take % 2);

        let mut peers: Vec<R> = Vec::with_capacity(max_num_peers_to_take);

        for i in offset_first_half..end_first_half {
            if let Some((_, peer)) = peer_map.get_index(i){
                peers.push(peer_conversion_function(peer))
            }
        }
        for i in offset_second_half..end_second_half {
            if let Some((_, peer)) = peer_map.get_index(i){
                peers.push(peer_conversion_function(peer))
            }
        }
        
        debug_assert_eq!(peers.len(), max_num_peers_to_take);

        peers
    }
}