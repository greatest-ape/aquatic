use std::net::SocketAddr;


#[derive(Clone)]
pub struct Config {
    pub address: SocketAddr,
    pub num_threads: usize,
    pub recv_buffer_size: usize,
    pub max_scrape_torrents: u8,
    pub max_response_peers: usize,
    pub statistics_interval: u64,
}


impl Default for Config {
    fn default() -> Self {
        Self {
            address: SocketAddr::from(([127, 0, 0, 1], 3000)),
            num_threads: 4,
            recv_buffer_size: 4096 * 16,
            max_scrape_torrents: 255,
            max_response_peers: 255,
            statistics_interval: 5,
        }
    }
}