use serde::{Serialize, Deserialize};

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(default)]
pub struct CpuPinningConfig {
    pub active: bool,
    pub offset: usize,
}

impl Default for CpuPinningConfig {
    fn default() -> Self {
        Self {
            active: false,
            offset: 0,
        }
    }
}
