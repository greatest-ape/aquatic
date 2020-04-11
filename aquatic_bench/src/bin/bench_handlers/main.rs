//! Benchmark announce and scrape handlers
//! 
//! Example summary output:
//! ```
//! ## Average results over 20 rounds in 4 threads
//! 
//! Connect handler:   2 543 896 requests/second,   393.10 ns/request
//! Announce handler:    382 055 requests/second,  2617.42 ns/request
//! Scrape handler:    1 168 651 requests/second,   855.69 ns/request
//! ```

use std::time::{Duration, Instant};
use std::io::Cursor;
use std::net::SocketAddr;
use std::sync::Arc;

use dashmap::DashMap;
use indicatif::{ProgressBar, ProgressStyle, ProgressIterator};
use num_format::{Locale, ToFormattedString};
use rand::{Rng, thread_rng, rngs::SmallRng, SeedableRng};

use aquatic::common::*;
use aquatic::config::Config;
use bittorrent_udp::converters::*;
use cli_helpers::run_app_with_cli_and_config;


mod announce;
mod common;
mod config;
mod connect;
mod scrape;

use common::*;
use config::BenchConfig;


#[global_allocator]
static GLOBAL: mimalloc::MiMalloc = mimalloc::MiMalloc;


fn print_results(
    request_type: &str,
    config: &BenchConfig,
    num_items: usize,
    duration: Duration,
) {
    let per_second = (
        ((config.num_threads as f64 * num_items as f64) / (duration.as_micros() as f64 / 1000000.0)
    ) as usize).to_formatted_string(&Locale::se);

    let time_per_request = duration.as_nanos() as f64 / (config.num_threads as f64 * num_items as f64);

    println!(
        "{} {:>10} requests/second, {:>8.2} ns/request",
        request_type,
        per_second,
        time_per_request,
    );
}


fn main(){
    run_app_with_cli_and_config::<BenchConfig>(
        "aquatic benchmarker",
        run
    )
}


fn run(bench_config: BenchConfig){
    let mut connect_data = (0usize, Duration::new(0, 0));
    let mut announce_data = (0usize, Duration::new(0, 0));
    let mut scrape_data = (0usize, Duration::new(0, 0));

    println!("# Benchmarking request handlers\n");

    // Benchmark connect handler
    {
        let requests = connect::create_requests();

        let requests: Vec<([u8; MAX_REQUEST_BYTES], SocketAddr)> = requests.into_iter()
            .map(|(request, src)| {
                let mut buffer = [0u8; MAX_REQUEST_BYTES];
                let mut cursor = Cursor::new(buffer.as_mut());

                request_to_bytes(&mut cursor, Request::Connect(request)).unwrap();

                (buffer, src)
            })
            .collect();

        let requests = Arc::new(requests);

        let pb = create_progress_bar("Connect handler", bench_config.num_rounds);

        for _ in (0..bench_config.num_rounds).progress_with(pb){
            let state = State::new();

            let handles: Vec<_> = (0..bench_config.num_threads).map(|_| {
                let requests = requests.clone();
                let state = state.clone();

                ::std::thread::spawn(|| connect::bench(state, requests))
            }).collect();

            for handle in handles {
                let (iterations, duration) = handle.join().unwrap();

                connect_data.0 += iterations;
                connect_data.1 += duration;
            }
        }
    }

    let mut rng = SmallRng::from_rng(thread_rng()).unwrap();
    let info_hashes = create_info_hashes(&mut rng);
    let config = Config::default();

    // Benchmark announce handler
    let last_torrents: Option<Arc<TorrentMap>> = {
        let requests = announce::create_requests(
            &mut rng,
            &info_hashes
        );

        // Create connections

        let connections = Arc::new(DashMap::new());
        let time = Time(Instant::now());

        for (request, src) in requests.iter() {
            let key = ConnectionKey {
                connection_id: request.connection_id,
                socket_addr: *src,
            };

            connections.insert(key, time);
        }

        let requests: Vec<([u8; MAX_REQUEST_BYTES], SocketAddr)> = requests.into_iter()
            .map(|(request, src)| {
                let mut buffer = [0u8; MAX_REQUEST_BYTES];
                let mut cursor = Cursor::new(buffer.as_mut());

                request_to_bytes(&mut cursor, Request::Announce(request)).unwrap();

                (buffer, src)
            })
            .collect();
        
        let requests = Arc::new(requests);

        let pb = create_progress_bar("Announce handler", bench_config.num_rounds);

        let mut last_torrents = None;

        for i in (0..bench_config.num_rounds).progress_with(pb){
            let mut state = State::new();

            state.connections = connections.clone();

            let handles: Vec<_> = (0..bench_config.num_threads).map(|_| {
                let requests = requests.clone();
                let state = state.clone();
                let config = config.clone();

                ::std::thread::spawn(move || announce::bench(&state, &config, requests))
            }).collect();

            for handle in handles {
                let (iterations, duration) = handle.join().unwrap();

                announce_data.0 += iterations;
                announce_data.1 += duration;
            }

            if i + 1 == bench_config.num_rounds {
                last_torrents = Some(state.torrents);
            }
        }

        last_torrents
    };

    // Benchmark scrape handler
    {
        let mut state = State::new();
        state.torrents = last_torrents.unwrap();

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

                request_to_bytes(&mut cursor, Request::Scrape(request)).unwrap();

                (buffer, src)
            })
            .collect();
        
        let requests = Arc::new(requests);

        let pb = create_progress_bar("Scrape handler", bench_config.num_rounds);

        for _ in (0..bench_config.num_rounds).progress_with(pb){
            let handles: Vec<_> = (0..bench_config.num_threads).map(|_| {
                let requests = requests.clone();
                let state = state.clone();

                ::std::thread::spawn(move || scrape::bench(&state, requests))
            }).collect();

            for handle in handles {
                let (iterations, duration) = handle.join().unwrap();

                scrape_data.0 += iterations;
                scrape_data.1 += duration;
            }
        }
    }

    println!(
        "\n## Average results over {} rounds in {} threads\n",
        bench_config.num_rounds,
        bench_config.num_threads
    );

    print_results("Connect handler: ", &bench_config, connect_data.0, connect_data.1);
    print_results("Announce handler:", &bench_config, announce_data.0, announce_data.1);
    print_results("Scrape handler:  ", &bench_config, scrape_data.0, scrape_data.1);
}


fn create_info_hashes(rng: &mut impl Rng) -> Vec<InfoHash> {
    let mut info_hashes = Vec::new();

    for _ in 0..common::NUM_INFO_HASHES {
        info_hashes.push(InfoHash(rng.gen()));
    }

    info_hashes
}


fn create_progress_bar(name: &str, iterations: u64) -> ProgressBar {
    let t = format!("{:<16} {}", name, "{wide_bar} {pos:>2}/{len:>2}");
    let style = ProgressStyle::default_bar().template(&t);

    ProgressBar::new(iterations).with_style(style)
}