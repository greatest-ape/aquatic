use std::fs::File;
use std::io::{BufRead, BufReader};
use std::path::PathBuf;

use hashbrown::HashSet;
use serde::{Deserialize, Serialize};

#[derive(Clone, Copy, Debug, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum AccessListMode {
    Allow,
    Deny,
    Ignore,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct AccessListConfig {
    pub path: PathBuf,
    pub mode: AccessListMode,
}

impl Default for AccessListConfig {
    fn default() -> Self {
        Self {
            path: "".into(),
            mode: AccessListMode::Ignore,
        }
    }
}

#[derive(Default)]
pub struct AccessList(HashSet<[u8; 20]>);

impl AccessList {
    fn parse_info_hash(line: String) -> anyhow::Result<[u8; 20]> {
        let mut bytes = [0u8; 20];

        hex::decode_to_slice(line, &mut bytes)?;

        Ok(bytes)
    }

    pub fn update_from_path(&mut self, path: &PathBuf) -> anyhow::Result<()> {
        let file = File::open(path)?;
        let reader = BufReader::new(file);

        let mut new_list = HashSet::new();

        for line in reader.lines() {
            new_list.insert(Self::parse_info_hash(line?)?);
        }

        self.0 = new_list;

        Ok(())
    }

    pub fn allows(&self, list_type: AccessListMode, info_hash_bytes: &[u8; 20]) -> bool {
        match list_type {
            AccessListMode::Allow => self.0.contains(info_hash_bytes),
            AccessListMode::Deny => !self.0.contains(info_hash_bytes),
            AccessListMode::Ignore => true,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_info_hash(){
        assert!(AccessList::parse_info_hash("aaaabbbbccccddddeeeeaaaabbbbccccddddeeee".to_owned()).is_ok());
        assert!(AccessList::parse_info_hash("aaaabbbbccccddddeeeeaaaabbbbccccddddeeeef".to_owned()).is_err());
        assert!(AccessList::parse_info_hash("aaaabbbbccccddddeeeeaaaabbbbccccddddeee".to_owned()).is_err());
        assert!(AccessList::parse_info_hash("aaaabbbbccccddddeeeeaaaabbbbccccddddeeeö".to_owned()).is_err());
    }
}