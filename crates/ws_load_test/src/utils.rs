use std::sync::Arc;

use rand::distributions::WeightedIndex;
use rand::prelude::*;
use rand_distr::Gamma;

use crate::common::*;
use crate::config::*;

pub fn create_random_request(
    config: &Config,
    state: &LoadTestState,
    rng: &mut impl Rng,
    peer_id: PeerId,
) -> InMessage {
    let weights = [
        config.torrents.weight_announce as u32,
        config.torrents.weight_scrape as u32,
    ];

    let items = [RequestType::Announce, RequestType::Scrape];

    let dist = WeightedIndex::new(&weights).expect("random request weighted index");

    match items[dist.sample(rng)] {
        RequestType::Announce => create_announce_request(config, state, rng, peer_id),
        RequestType::Scrape => create_scrape_request(config, state, rng),
    }
}

#[inline]
fn create_announce_request(
    config: &Config,
    state: &LoadTestState,
    rng: &mut impl Rng,
    peer_id: PeerId,
) -> InMessage {
    let (event, bytes_left) = {
        if rng.gen_bool(config.torrents.peer_seeder_probability) {
            (AnnounceEvent::Completed, 0)
        } else {
            (AnnounceEvent::Started, 50)
        }
    };

    let info_hash_index = select_info_hash_index(config, &state, rng);

    let mut offers = Vec::with_capacity(config.torrents.offers_per_request);

    for _ in 0..config.torrents.offers_per_request {
        offers.push(AnnounceRequestOffer {
            offer_id: OfferId(rng.gen()),
            offer: JsonValue(::serde_json::json!(
                {"sdp": "abcdefg-abcdefg-abcdefg-abcdefg-abcdefg-abcdefg-abcdefg-abcdefg-abcdefg-abcdefg-abcdefg-abcdefg-abcdefg-abcdefg-abcdefg-"}
            )),
        })
    }

    InMessage::AnnounceRequest(AnnounceRequest {
        action: AnnounceAction,
        info_hash: state.info_hashes[info_hash_index],
        peer_id,
        bytes_left: Some(bytes_left),
        event: Some(event),
        numwant: Some(offers.len()),
        offers: Some(offers),
        answer: None,
        to_peer_id: None,
        offer_id: None,
    })
}

#[inline]
fn create_scrape_request(config: &Config, state: &LoadTestState, rng: &mut impl Rng) -> InMessage {
    let mut scrape_hashes = Vec::with_capacity(5);

    for _ in 0..5 {
        let info_hash_index = select_info_hash_index(config, &state, rng);

        scrape_hashes.push(state.info_hashes[info_hash_index]);
    }

    InMessage::ScrapeRequest(ScrapeRequest {
        action: ScrapeAction,
        info_hashes: Some(ScrapeRequestInfoHashes::Multiple(scrape_hashes)),
    })
}

#[inline]
fn select_info_hash_index(config: &Config, state: &LoadTestState, rng: &mut impl Rng) -> usize {
    gamma_usize(rng, &state.gamma, config.torrents.number_of_torrents - 1)
}

#[inline]
fn gamma_usize(rng: &mut impl Rng, gamma: &Arc<Gamma<f64>>, max: usize) -> usize {
    let p: f64 = gamma.sample(rng);
    let p = (p.min(101.0f64) - 1.0) / 100.0;

    (p * max as f64) as usize
}
