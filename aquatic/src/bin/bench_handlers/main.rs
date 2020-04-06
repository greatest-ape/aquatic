//! Benchmark announce and scrape handlers
//! 
//! Example summary output:
//! ```
//! # Average results over 100 rounds
//! connect handler:   3 201 147 requests/second,   312.66 ns/request
//! announce handler:    330 958 requests/second,  3029.13 ns/request
//! scrape handler:    1 242 478 requests/second,   805.62 ns/request
//! ```

use std::time::{Duration, Instant};

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
    let num_rounds = 100;

    let mut connect_data = (0.0, 0.0);
    let mut announce_data = (0.0, 0.0);
    let mut scrape_data = (0.0, 0.0);

    {
        let requests = connect::create_requests();

        ::std::thread::sleep(Duration::from_secs(1));

        for round in 0..num_rounds {
            println!("# Round {}/{}\n", round + 1, num_rounds);

            let d = connect::bench(requests.clone());
            connect_data.0 += d.0;
            connect_data.1 += d.1;
        }
    }

    let mut rng = SmallRng::from_rng(thread_rng()).unwrap();
    let info_hashes = create_info_hashes(&mut rng);

    let state_for_scrape: State = {
        let requests = announce::create_requests(
            &mut rng,
            &info_hashes
        );

        let mut state_for_scrape = State::new();

        ::std::thread::sleep(Duration::from_secs(1));

        for round in 0..num_rounds {
            println!("# Round {}/{}\n", round + 1, num_rounds);

            let state = State::new();

            let time = Time(Instant::now());

            for (request, src) in requests.iter() {
                let key = ConnectionKey {
                    connection_id: request.connection_id,
                    socket_addr: *src,
                };

                state.connections.insert(key, time);
            }

            let d = announce::bench(&state, requests.clone());
            announce_data.0 += d.0;
            announce_data.1 += d.1;

            if round == num_rounds - 1 {
                state_for_scrape = state.clone();
            }
        }

        state_for_scrape
    };

    state_for_scrape.connections.clear();

    {
        let state = state_for_scrape;

        let requests = scrape::create_requests(&mut rng, &info_hashes);

        let time = Time(Instant::now());

        for (request, src) in requests.iter() {
            let key = ConnectionKey {
                connection_id: request.connection_id,
                socket_addr: *src,
            };

            state.connections.insert(key, time);
        }

        ::std::thread::sleep(Duration::from_secs(1));

        for round in 0..num_rounds {
            println!("# Round {}/{}\n", round + 1, num_rounds);

            let d = scrape::bench(&state, requests.clone());
            scrape_data.0 += d.0;
            scrape_data.1 += d.1;

            println!();
        }
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