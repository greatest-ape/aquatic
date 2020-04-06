//! Benchmark announce and scrape handlers
//! 
//! Example summary output:
//! ```
//! # Average results over 20 rounds
//! 
//! connect handler:   3 365 415 requests/second,   297.41 ns/request
//! announce handler:    346 650 requests/second,  2921.76 ns/request
//! scrape handler:    1 313 100 requests/second,   762.47 ns/request
//! ```

use std::time::Duration;

use num_format::{Locale, ToFormattedString};
use rand::{Rng, thread_rng, rngs::SmallRng, SeedableRng};

use aquatic::common::*;


mod announce;
mod common;
mod connect;
mod scrape;


#[global_allocator]
static GLOBAL: mimalloc::MiMalloc = mimalloc::MiMalloc;


macro_rules! print_results {
    ($request_type:expr, $num_rounds:expr, $data:expr) => {
        let per_second = (
            ($data.0 / ($num_rounds as f64)
        ) as usize).to_formatted_string(&Locale::se);

        println!(
            "{} {:>10} requests/second, {:>8.2} ns/request",
            $request_type,
            per_second,
            $data.1 / ($num_rounds as f64)
        );
    };
}


fn main(){
    let num_rounds = 20;

    let mut connect_data = (0.0, 0.0);
    let mut announce_data = (0.0, 0.0);
    let mut scrape_data = (0.0, 0.0);

    for round in 0..num_rounds {
        println!("# Round {}/{}\n", round + 1, num_rounds);

        let d = connect::bench();
        connect_data.0 += d.0;
        connect_data.1 += d.1;

        println!("");

        ::std::thread::sleep(Duration::from_millis(100));

        let mut rng = SmallRng::from_rng(thread_rng()).unwrap();
        let info_hashes = create_info_hashes(&mut rng);
        let state = State::new();

        let d = announce::bench(&mut rng, &state, &info_hashes);
        announce_data.0 += d.0;
        announce_data.1 += d.1;

        state.connections.clear();

        println!("");

        ::std::thread::sleep(Duration::from_millis(100));

        let d = scrape::bench(&mut rng, &state, &info_hashes);
        scrape_data.0 += d.0;
        scrape_data.1 += d.1;

        println!();
    }

    println!("# Average results over {} rounds\n", num_rounds);

    print_results!("connect handler: ", num_rounds, connect_data);
    print_results!("announce handler:", num_rounds, announce_data);
    print_results!("scrape handler:  ", num_rounds, scrape_data);
}


fn create_info_hashes(rng: &mut impl Rng) -> Vec<InfoHash> {
    let mut info_hashes = Vec::new();

    for _ in 0..common::NUM_INFO_HASHES {
        info_hashes.push(InfoHash(rng.gen()));
    }

    info_hashes
}