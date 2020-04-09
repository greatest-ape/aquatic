//! Benchmark announce and scrape handlers
//! 
//! Example summary output:
//! ```
//! ## Average results over 50 rounds
//! 
//! Connect handler:   2 530 072 requests/second,   395.38 ns/request
//! Announce handler:    309 719 requests/second,  3229.87 ns/request
//! Scrape handler:      595 259 requests/second,  1680.01 ns/request
//! ```

use std::time::{Duration, Instant};
use std::io::Cursor;
use std::net::SocketAddr;

use indicatif::{ProgressBar, ProgressStyle, ProgressIterator};
use num_format::{Locale, ToFormattedString};
use rand::{Rng, thread_rng, rngs::SmallRng, SeedableRng};

use aquatic::common::*;
use aquatic::config::Config;
use bittorrent_udp::converters::*;


mod announce;
mod common;
mod connect;
mod scrape;

use common::*;


#[global_allocator]
static GLOBAL: mimalloc::MiMalloc = mimalloc::MiMalloc;


fn print_results(
    request_type: &str,
    num_rounds: u64,
    data: (f64, f64)
) {
    let per_second = (
        (data.0 / (num_rounds as f64)
    ) as usize).to_formatted_string(&Locale::se);

    println!(
        "{} {:>10} requests/second, {:>8.2} ns/request",
        request_type,
        per_second,
        data.1 / (num_rounds as f64)
    );
}


fn main(){
    let num_rounds = 1;

    let mut connect_data = (0.0, 0.0);
    let mut announce_data = (0.0, 0.0);
    let mut scrape_data = (0.0, 0.0);

    fn create_progress_bar(name: &str, iterations: u64) -> ProgressBar {
        let t = format!("{:<16} {}", name, "{wide_bar} {pos:>2}/{len:>2}");
        let style = ProgressStyle::default_bar().template(&t);

        ProgressBar::new(iterations).with_style(style)
    }

    println!("# Benchmarking request handlers\n");

    // Benchmark connect handler
    {
        let requests = connect::create_requests();

        let requests: Vec<([u8; MAX_REQUEST_BYTES], SocketAddr)> = requests.into_iter()
            .map(|(request, src)| {
                let mut buffer = [0u8; MAX_REQUEST_BYTES];
                let mut cursor = Cursor::new(buffer.as_mut());

                request_to_bytes(&mut cursor, Request::Connect(request));

                (buffer, src)
            })
            .collect();

        ::std::thread::sleep(Duration::from_secs(1));

        let pb = create_progress_bar("Connect handler", num_rounds);

        for _ in (0..num_rounds).progress_with(pb){
            let requests = requests.clone();

            ::std::thread::sleep(Duration::from_millis(200));

            let d = connect::bench(requests);

            ::std::thread::sleep(Duration::from_millis(200));

            connect_data.0 += d.0;
            connect_data.1 += d.1;
        }
    }

    let mut rng = SmallRng::from_rng(thread_rng()).unwrap();
    let info_hashes = create_info_hashes(&mut rng);
    let config = Config::default();

    // Benchmark announce handler
    let last_state: State = {
        let state = State::new();

        let requests = announce::create_requests(
            &mut rng,
            &info_hashes
        );

        // Create connections in state

        let time = Time(Instant::now());

        for (request, src) in requests.iter() {
            let key = ConnectionKey {
                connection_id: request.connection_id,
                socket_addr: *src,
            };

            state.connections.insert(key, time);
        }

        let requests: Vec<([u8; MAX_REQUEST_BYTES], SocketAddr)> = requests.into_iter()
            .map(|(request, src)| {
                let mut buffer = [0u8; MAX_REQUEST_BYTES];
                let mut cursor = Cursor::new(buffer.as_mut());

                request_to_bytes(&mut cursor, Request::Announce(request));

                (buffer, src)
            })
            .collect();

        ::std::thread::sleep(Duration::from_secs(1));

        let pb = create_progress_bar("Announce handler", num_rounds);

        for _ in (0..num_rounds).progress_with(pb) {
            let requests = requests.clone();
            state.torrents.clear();
            state.torrents.shrink_to_fit();

            ::std::thread::sleep(Duration::from_millis(200));

            let d = announce::bench(&state, &config, requests);

            ::std::thread::sleep(Duration::from_millis(200));

            announce_data.0 += d.0;
            announce_data.1 += d.1;
        }

        state
    };

    // Benchmark scrape handler
    {
        let state = last_state;

        state.connections.clear();

        let requests = scrape::create_requests(&mut rng, &info_hashes);

        // Create connections in state

        let time = Time(Instant::now());

        for (request, src) in requests.iter() {
            let key = ConnectionKey {
                connection_id: request.connection_id,
                socket_addr: *src,
            };

            state.connections.insert(key, time);
        }

        let requests: Vec<([u8; MAX_REQUEST_BYTES], SocketAddr)> = requests.into_iter()
            .map(|(request, src)| {
                let mut buffer = [0u8; MAX_REQUEST_BYTES];
                let mut cursor = Cursor::new(buffer.as_mut());

                request_to_bytes(&mut cursor, Request::Scrape(request));

                (buffer, src)
            })
            .collect();

        ::std::thread::sleep(Duration::from_secs(1));

        let pb = create_progress_bar("Scrape handler", num_rounds);

        for _ in (0..num_rounds).progress_with(pb) {
            let requests = requests.clone();

            ::std::thread::sleep(Duration::from_millis(200));

            let d = scrape::bench(&state, requests);

            ::std::thread::sleep(Duration::from_millis(200));

            scrape_data.0 += d.0;
            scrape_data.1 += d.1;
        }
    }

    println!("\n## Average results over {} rounds\n", num_rounds);

    print_results("Connect handler: ", num_rounds, connect_data);
    print_results("Announce handler:", num_rounds, announce_data);
    print_results("Scrape handler:  ", num_rounds, scrape_data);
}


fn create_info_hashes(rng: &mut impl Rng) -> Vec<InfoHash> {
    let mut info_hashes = Vec::new();

    for _ in 0..common::NUM_INFO_HASHES {
        info_hashes.push(InfoHash(rng.gen()));
    }

    info_hashes
}
