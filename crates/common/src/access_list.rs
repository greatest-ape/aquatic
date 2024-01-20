use std::fs::File;
use std::io::{BufRead, BufReader};
use std::path::PathBuf;
use std::sync::Arc;

use anyhow::Context;
use aquatic_toml_config::TomlConfig;
use arc_swap::{ArcSwap, Cache};
use hashbrown::HashSet;
use serde::{Deserialize, Serialize};

/// Access list mode. Available modes are allow, deny and off.
#[derive(Clone, Copy, Debug, PartialEq, TomlConfig, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum AccessListMode {
    /// Only serve torrents with info hash present in file
    Allow,
    /// Do not serve torrents if info hash present in file
    Deny,
    /// Turn off access list functionality
    Off,
}

impl AccessListMode {
    pub fn is_on(&self) -> bool {
        !matches!(self, Self::Off)
    }
}

#[derive(Clone, Debug, PartialEq, TomlConfig, Deserialize, Serialize)]
#[serde(default, deny_unknown_fields)]
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
            path: "./access-list.txt".into(),
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
            let line = line.trim();

            if line.is_empty() {
                continue;
            }

            new_list
                .insert_from_line(line)
                .with_context(|| format!("Invalid line in access list: {}", line))?;
        }

        Ok(new_list)
    }

    pub fn allows(&self, mode: AccessListMode, info_hash: &[u8; 20]) -> bool {
        match mode {
            AccessListMode::Allow => self.0.contains(info_hash),
            AccessListMode::Deny => !self.0.contains(info_hash),
            AccessListMode::Off => true,
        }
    }

    #[allow(clippy::len_without_is_empty)]
    pub fn len(&self) -> usize {
        self.0.len()
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
            AccessListMode::Allow => self.load().0.contains(info_hash_bytes),
            AccessListMode::Deny => !self.load().0.contains(info_hash_bytes),
            AccessListMode::Off => true,
        }
    }
}

pub fn create_access_list_cache(arc_swap: &Arc<AccessListArcSwap>) -> AccessListCache {
    Cache::from(Arc::clone(arc_swap))
}

pub fn update_access_list(
    config: &AccessListConfig,
    access_list: &Arc<AccessListArcSwap>,
) -> anyhow::Result<()> {
    if config.mode.is_on() {
        match access_list.update(config) {
            Ok(()) => {
                ::log::info!("Access list updated")
            }
            Err(err) => {
                ::log::error!("Updating access list failed: {:#}", err);

                return Err(err);
            }
        }
    }

    Ok(())
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

        assert!(f("aaaabbbbccccddddeeeeaaaabbbbccccddddeeee").is_ok());
        assert!(f("aaaabbbbccccddddeeeeaaaabbbbccccddddeeeef").is_err());
        assert!(f("aaaabbbbccccddddeeeeaaaabbbbccccddddeee").is_err());
        assert!(f("aaaabbbbccccddddeeeeaaaabbbbccccddddeeeö").is_err());
    }

    #[test]
    fn test_cache_allows() {
        let mut access_list = AccessList::default();

        let a = parse_info_hash("aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa").unwrap();
        let b = parse_info_hash("bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb").unwrap();
        let c = parse_info_hash("cccccccccccccccccccccccccccccccccccccccc").unwrap();

        access_list.0.insert(a);
        access_list.0.insert(b);

        let access_list = Arc::new(ArcSwap::new(Arc::new(access_list)));

        let mut access_list_cache = Cache::new(Arc::clone(&access_list));

        assert!(access_list_cache.load().allows(AccessListMode::Allow, &a));
        assert!(access_list_cache.load().allows(AccessListMode::Allow, &b));
        assert!(!access_list_cache.load().allows(AccessListMode::Allow, &c));

        assert!(!access_list_cache.load().allows(AccessListMode::Deny, &a));
        assert!(!access_list_cache.load().allows(AccessListMode::Deny, &b));
        assert!(access_list_cache.load().allows(AccessListMode::Deny, &c));

        assert!(access_list_cache.load().allows(AccessListMode::Off, &a));
        assert!(access_list_cache.load().allows(AccessListMode::Off, &b));
        assert!(access_list_cache.load().allows(AccessListMode::Off, &c));

        access_list.store(Arc::new(AccessList::default()));

        assert!(access_list_cache.load().allows(AccessListMode::Deny, &a));
        assert!(access_list_cache.load().allows(AccessListMode::Deny, &b));
    }
}
