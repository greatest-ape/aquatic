use std::sync::Arc;

use rand::distributions::WeightedIndex;
use rand::prelude::*;
use rand_distr::Pareto;

use crate::common::*;
use crate::config::*;

pub fn create_random_request(
    config: &Config,
    state: &LoadTestState,
    rng: &mut impl Rng,
) -> Request {
    let weights = [
        config.torrents.weight_announce as u32,
        config.torrents.weight_scrape as u32,
    ];

    let items = [RequestType::Announce, RequestType::Scrape];

    let dist = WeightedIndex::new(&weights).expect("random request weighted index");

    match items[dist.sample(rng)] {
        RequestType::Announce => create_announce_request(config, state, rng),
        RequestType::Scrape => create_scrape_request(config, state, rng),
    }
}

#[inline]
fn create_announce_request(config: &Config, state: &LoadTestState, rng: &mut impl Rng) -> Request {
    let (event, bytes_left) = {
        if rng.gen_bool(config.torrents.peer_seeder_probability) {
            (AnnounceEvent::Completed, 0)
        } else {
            (AnnounceEvent::Started, 50)
        }
    };

    let info_hash_index = select_info_hash_index(config, &state, rng);

    Request::Announce(AnnounceRequest {
        info_hash: state.info_hashes[info_hash_index],
        peer_id: PeerId(rng.gen()),
        bytes_left,
        event,
        key: None,
        numwant: None,
        compact: true,
        port: rng.gen(),
    })
}

#[inline]
fn create_scrape_request(config: &Config, state: &LoadTestState, rng: &mut impl Rng) -> Request {
    let mut scrape_hashes = Vec::with_capacity(5);

    for _ in 0..5 {
        let info_hash_index = select_info_hash_index(config, &state, rng);

        scrape_hashes.push(state.info_hashes[info_hash_index]);
    }

    Request::Scrape(ScrapeRequest {
        info_hashes: scrape_hashes,
    })
}

#[inline]
fn select_info_hash_index(config: &Config, state: &LoadTestState, rng: &mut impl Rng) -> usize {
    pareto_usize(rng, &state.pareto, config.torrents.number_of_torrents - 1)
}

#[inline]
fn pareto_usize(rng: &mut impl Rng, pareto: &Arc<Pareto<f64>>, max: usize) -> usize {
    let p: f64 = pareto.sample(rng);
    let p = (p.min(101.0f64) - 1.0) / 100.0;

    (p * max as f64) as usize
}
