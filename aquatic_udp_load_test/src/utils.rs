use std::sync::Arc;

use rand::prelude::*;
use rand_distr::Pareto;

use aquatic_udp_protocol::*;

use crate::config::Config;
use crate::common::*;

pub fn create_torrent_peer(
    config: &Config,
    rng: &mut impl Rng,
    pareto: Pareto<f64>,
    info_hashes: &Arc<Vec<InfoHash>>,
    connection_id: ConnectionId,
) -> TorrentPeer {
    let num_scape_hashes = rng.gen_range(1..config.handler.scrape_max_torrents);

    let mut scrape_hash_indeces = Vec::new();

    for _ in 0..num_scape_hashes {
        scrape_hash_indeces.push(select_info_hash_index(config, rng, pareto))
    }

    let info_hash_index = select_info_hash_index(config, rng, pareto);

    TorrentPeer {
        info_hash: info_hashes[info_hash_index],
        scrape_hash_indeces,
        connection_id,
        peer_id: generate_peer_id(),
        port: Port(rand::random()),
    }
}

fn select_info_hash_index(config: &Config, rng: &mut impl Rng, pareto: Pareto<f64>) -> usize {
    pareto_usize(rng, pareto, config.handler.number_of_torrents - 1)
}

pub fn pareto_usize(rng: &mut impl Rng, pareto: Pareto<f64>, max: usize) -> usize {
    let p: f64 = rng.sample(pareto);
    let p = (p.min(101.0f64) - 1.0) / 100.0;

    (p * max as f64) as usize
}

pub fn generate_peer_id() -> PeerId {
    PeerId(random_20_bytes())
}

pub fn generate_info_hash() -> InfoHash {
    InfoHash(random_20_bytes())
}

pub fn generate_transaction_id(rng: &mut impl Rng) -> TransactionId {
    TransactionId(rng.gen())
}

pub fn create_connect_request(transaction_id: TransactionId) -> Request {
    (ConnectRequest { transaction_id }).into()
}

// Don't use SmallRng here for now
fn random_20_bytes() -> [u8; 20] {
    let mut bytes = [0; 20];

    thread_rng().fill_bytes(&mut bytes[..]);

    bytes
}
