use std::sync::Arc;

use rand_distr::{Standard, Pareto};
use rand::prelude::*;

use crate::common::*;


pub fn select_info_hash_index(
    config: &Config,
    state: &LoadTestState,
    rng: &mut impl Rng,
) -> usize {
    pareto_usize(rng, &state.pareto, config.handler.number_of_torrents - 1)
}


pub fn pareto_usize(
    rng: &mut impl Rng,
    pareto: &Arc<Pareto<f64>>,
    max: usize,
) -> usize {
    let p: f64 = pareto.sample(rng);
    let p = (p.min(101.0f64) - 1.0) / 100.0;

    (p * max as f64) as usize
}


pub fn generate_info_hash(rng: &mut impl Rng) -> InfoHash {
    InfoHash(rng.gen())
}
