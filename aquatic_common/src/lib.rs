use std::fs::File;
use std::io::{BufRead, BufReader};
use std::net::IpAddr;
use std::path::PathBuf;
use std::time::{Duration, Instant};

use hashbrown::HashSet;
use indexmap::IndexMap;
use rand::Rng;
use serde::{Deserialize, Serialize};

#[derive(Clone, Copy, Debug, Serialize, Deserialize)]
pub enum AccessListType {
    Allow,
    Deny,
    Ignore
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct AccessListConfig {
    pub path: PathBuf,
    pub list_type: AccessListType,
}

impl Default for AccessListConfig {
    fn default() -> Self {
        Self {
            path: "".into(),
            list_type: AccessListType::Ignore,
        }
    }
}

#[derive(Default)]
pub struct AccessList(HashSet<[u8; 20]>);

impl AccessList {
    fn parse_line_to_info_hash(line: String) -> anyhow::Result<[u8; 20]> {
        let mut count = 0usize;
        let mut bytes = [0u8; 20];

        for (byte, c) in bytes.iter_mut().zip(line.chars()) {
            *byte = c as u8;
            count += 1;
        }

        if count == 20 {
            Ok(bytes)
        } else {
            Err(anyhow::anyhow!("Info hash length only {} bytes: {}", count, line))
        }
    }

    pub fn update_from_path(&mut self, path: &PathBuf) -> anyhow::Result<()> {
        let file = File::open(path)?;
        let reader = BufReader::new(file);

        self.0.clear();

        for line in reader.lines() {
            self.0.insert(Self::parse_line_to_info_hash(line?)?);
        }


        Ok(())
    }

    pub fn allows(&self, list_type: AccessListType, info_hash_bytes: &[u8; 20]) -> bool {
        match list_type {
            AccessListType::Allow => {
                self.0.contains(info_hash_bytes)
            }
            AccessListType::Deny => {
                !self.0.contains(info_hash_bytes)
            }
            AccessListType::Ignore => {
                true
            }
        }
    }
}

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
/// Might return one less peer than wanted since sender is filtered out.
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
    F: Fn(&V) -> R,
{
    let peer_map_len = peer_map.len();

    if peer_map_len <= max_num_peers_to_take + 1 {
        peer_map
            .iter()
            .filter_map(|(k, v)| {
                if *k == sender_peer_map_key {
                    None
                } else {
                    Some(peer_conversion_function(v))
                }
            })
            .collect()
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

#[inline]
pub fn convert_ipv4_mapped_ipv6(ip_address: IpAddr) -> IpAddr {
    if let IpAddr::V6(ip) = ip_address {
        if let [0, 0, 0, 0, 0, 0xffff, ..] = ip.segments() {
            ip.to_ipv4().expect("convert ipv4-mapped ip").into()
        } else {
            ip_address
        }
    } else {
        ip_address
    }
}
