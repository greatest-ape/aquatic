use rand::prelude::*;
use rand_distr::Gamma;

use aquatic_udp_protocol::*;

pub fn pareto_usize(rng: &mut impl Rng, pareto: Gamma<f64>, max: usize) -> usize {
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
