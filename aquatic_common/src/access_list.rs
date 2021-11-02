use std::fs::File;
use std::io::{BufRead, BufReader};
use std::path::PathBuf;
use std::sync::Arc;

use anyhow::Context;
use arc_swap::{ArcSwap, Cache};
use hashbrown::HashSet;
use serde::{Deserialize, Serialize};

#[derive(Clone, Copy, Debug, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum AccessListMode {
    /// Only serve torrents with info hash present in file
    White,
    /// Do not serve torrents if info hash present in file
    Black,
    /// Turn off access list functionality
    Off,
}

impl AccessListMode {
    pub fn is_on(&self) -> bool {
        !matches!(self, Self::Off)
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct AccessListConfig {
    pub mode: AccessListMode,
    /// Path to access list file consisting of newline-separated hex-encoded info hashes.
    ///
    /// If using chroot mode, path must be relative to new root.
    pub path: PathBuf,
}

impl Default for AccessListConfig {
    fn default() -> Self {
        Self {
            path: "".into(),
            mode: AccessListMode::Off,
        }
    }
}

#[derive(Default, Clone)]
pub struct AccessList(HashSet<[u8; 20]>);

impl AccessList {
    pub fn insert_from_line(&mut self, line: &str) -> anyhow::Result<()> {
        self.0.insert(parse_info_hash(line)?);

        Ok(())
    }

    pub fn create_from_path(path: &PathBuf) -> anyhow::Result<Self> {
        let file = File::open(path)?;
        let reader = BufReader::new(file);

        let mut new_list = Self::default();

        for line in reader.lines() {
            let line = line?;

            new_list
                .insert_from_line(&line)
                .with_context(|| format!("Invalid line in access list: {}", line))?;
        }

        Ok(new_list)
    }

    pub fn allows(&self, mode: AccessListMode, info_hash: &[u8; 20]) -> bool {
        match mode {
            AccessListMode::White => self.0.contains(info_hash),
            AccessListMode::Black => !self.0.contains(info_hash),
            AccessListMode::Off => true,
        }
    }
}

pub trait AccessListQuery {
    fn update(&self, config: &AccessListConfig) -> anyhow::Result<()>;
    fn allows(&self, list_mode: AccessListMode, info_hash_bytes: &[u8; 20]) -> bool;
}

pub type AccessListArcSwap = ArcSwap<AccessList>;
pub type AccessListCache = Cache<Arc<AccessListArcSwap>, Arc<AccessList>>;

impl AccessListQuery for AccessListArcSwap {
    fn update(&self, config: &AccessListConfig) -> anyhow::Result<()> {
        self.store(Arc::new(AccessList::create_from_path(&config.path)?));

        Ok(())
    }

    fn allows(&self, mode: AccessListMode, info_hash_bytes: &[u8; 20]) -> bool {
        match mode {
            AccessListMode::White => self.load().0.contains(info_hash_bytes),
            AccessListMode::Black => !self.load().0.contains(info_hash_bytes),
            AccessListMode::Off => true,
        }
    }
}

pub fn create_access_list_cache(arc_swap: &Arc<AccessListArcSwap>) -> AccessListCache {
    Cache::from(Arc::clone(arc_swap))
}

fn parse_info_hash(line: &str) -> anyhow::Result<[u8; 20]> {
    let mut bytes = [0u8; 20];

    hex::decode_to_slice(line, &mut bytes)?;

    Ok(bytes)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_info_hash() {
        let f = parse_info_hash;

        assert!(f("aaaabbbbccccddddeeeeaaaabbbbccccddddeeee".into()).is_ok());
        assert!(f("aaaabbbbccccddddeeeeaaaabbbbccccddddeeeef".into()).is_err());
        assert!(f("aaaabbbbccccddddeeeeaaaabbbbccccddddeee".into()).is_err());
        assert!(f("aaaabbbbccccddddeeeeaaaabbbbccccddddeeeö".into()).is_err());
    }
}
