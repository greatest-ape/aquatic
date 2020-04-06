use std::net::SocketAddr;


#[derive(Clone)]
pub struct Config {
    pub address: SocketAddr,
    pub recv_buffer_size: usize,
    pub max_scrape_torrents: u8,
    pub max_response_peers: usize,
}


impl Default for Config {
    fn default() -> Self {
        Self {
            address: SocketAddr::from(([127, 0, 0, 1], 3000)),
            recv_buffer_size: 4096 * 16,
            max_scrape_torrents: 255,
            max_response_peers: 255,
        }
    }
}