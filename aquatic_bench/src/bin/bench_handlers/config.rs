use serde::{Serialize, Deserialize};


#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct BenchConfig {
    pub num_rounds: u64,
    pub num_threads: u64,
}


impl Default for BenchConfig {
    fn default() -> Self {
        Self {
            num_rounds: 20,
            num_threads: 4,
        }
    }
}