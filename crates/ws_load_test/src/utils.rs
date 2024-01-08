use std::sync::Arc;

use rand::prelude::*;
use rand_distr::Gamma;

use crate::common::*;
use crate::config::*;

#[inline]
pub fn select_info_hash_index(config: &Config, state: &LoadTestState, rng: &mut impl Rng) -> usize {
    gamma_usize(rng, &state.gamma, config.torrents.number_of_torrents - 1)
}

#[inline]
fn gamma_usize(rng: &mut impl Rng, gamma: &Arc<Gamma<f64>>, max: usize) -> usize {
    let p: f64 = gamma.sample(rng);
    let p = (p.min(101.0f64) - 1.0) / 100.0;

    (p * max as f64) as usize
}
