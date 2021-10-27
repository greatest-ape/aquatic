use serde::{Serialize, Deserialize};

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(default)]
pub struct CoreAffinityConfig {
    pub set_affinities: bool,
    pub offset: usize,
}

impl Default for CoreAffinityConfig {
    fn default() -> Self {
        Self {
            set_affinities: false,
            offset: 0,
        }
    }
}
