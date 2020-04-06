//! Benchmark announce and scrape handlers

use std::time::Duration;

use rand::{Rng, thread_rng, rngs::SmallRng, SeedableRng};

use aquatic::common::*;


mod announce;
mod common;
mod connect;
mod scrape;


#[global_allocator]
static GLOBAL: mimalloc::MiMalloc = mimalloc::MiMalloc;


fn main(){
    connect::bench();

    println!("");

    ::std::thread::sleep(Duration::from_secs(1));

    let mut rng = SmallRng::from_rng(thread_rng()).unwrap();
    let info_hashes = create_info_hashes(&mut rng);
    let state = State::new();

    announce::bench(&mut rng, &state, &info_hashes);

    state.connections.clear();

    println!("");

    ::std::thread::sleep(Duration::from_secs(1));

    scrape::bench(&mut rng, &state, &info_hashes);
}


fn create_info_hashes(rng: &mut impl Rng) -> Vec<InfoHash> {
    let mut info_hashes = Vec::new();

    for _ in 0..common::NUM_INFO_HASHES {
        info_hashes.push(InfoHash(rng.gen()));
    }

    info_hashes
}