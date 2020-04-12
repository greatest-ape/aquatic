use serde::{Serialize, Deserialize};


#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct BenchConfig {
    pub num_rounds: usize,
    pub num_threads: usize,
    pub num_connect_requests: usize,
}


impl Default for BenchConfig {
    fn default() -> Self {
        Self {
            num_rounds: 20,
            num_threads: 2,
            num_connect_requests: 5_000_000,
        }
    }
}