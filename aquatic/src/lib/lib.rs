use std::sync::atomic::Ordering;
use std::time::Duration;

use histogram::Histogram;

pub mod common;
pub mod config;
pub mod handlers;
pub mod network;
pub mod tasks;

use config::Config;
use common::State;


pub fn run(){
    let config = Config::default();
    let state = State::new();

    for i in 0..config.num_threads {
        let state = state.clone();
        let config = config.clone();

        ::std::thread::spawn(move || {
            network::run_event_loop(state, config, i);
        });
    }

    {
        let state = state.clone();

        ::std::thread::spawn(move || {
            let interval = config.statistics_interval;

            loop {
                ::std::thread::sleep(Duration::from_secs(interval));

                let requests_received: f64 = state.statistics.requests_received
                    .fetch_and(0, Ordering::SeqCst) as f64;
                let responses_sent: f64 = state.statistics.responses_sent
                    .fetch_and(0, Ordering::SeqCst) as f64;
                let bytes_received: f64 = state.statistics.bytes_received
                    .fetch_and(0, Ordering::SeqCst) as f64;
                let bytes_sent: f64 = state.statistics.bytes_sent
                    .fetch_and(0, Ordering::SeqCst) as f64;

                let requests_per_second = requests_received / interval as f64;
                let responses_per_second: f64 = responses_sent / interval as f64;
                let bytes_received_per_second: f64 = bytes_received / interval as f64;
                let bytes_sent_per_second: f64 = bytes_sent / interval as f64;

                let readable_events: f64 = state.statistics.readable_events
                    .fetch_and(0, Ordering::SeqCst) as f64;
                let requests_per_readable_event = if readable_events == 0.0 {
                    0.0
                } else {
                    requests_received / readable_events
                };

                println!(
                    "stats: {:.2} requests/second, {:.2} responses/second, {:.2} requests/readable event",
                    requests_per_second,
                    responses_per_second,
                    requests_per_readable_event
                );

                println!(
                    "bandwidth: {:7.2} Mbit/s in, {:7.2} Mbit/s out",
                    bytes_received_per_second * 8.0 / 1_000_000.0,
                    bytes_sent_per_second * 8.0 / 1_000_000.0,
                );

                let mut peers_per_torrent = Histogram::new();

                for torrent in state.torrents.iter(){
                    let num_seeders = torrent.num_seeders.load(Ordering::SeqCst);
                    let num_leechers = torrent.num_leechers.load(Ordering::SeqCst);

                    let num_peers = (num_seeders + num_leechers) as u64;

                    if let Err(err) = peers_per_torrent.increment(num_peers){
                        eprintln!("error incrementing peers_per_torrent histogram: {}", err)
                    }
                }

                if peers_per_torrent.entries() != 0 {
                    println!(
                        "peers per torrent: min: {}, p50: {}, p75: {}, p90: {}, p99: {}, p999: {}, max: {}",
                        peers_per_torrent.minimum().unwrap(),
                        peers_per_torrent.percentile(50.0).unwrap(),
                        peers_per_torrent.percentile(75.0).unwrap(),
                        peers_per_torrent.percentile(90.0).unwrap(),
                        peers_per_torrent.percentile(99.0).unwrap(),
                        peers_per_torrent.percentile(99.9).unwrap(),
                        peers_per_torrent.maximum().unwrap(),
                    );
                }

                println!();
            }
        });
    }

    loop {
        ::std::thread::sleep(Duration::from_secs(30));

        tasks::clean_connections(&state);
        tasks::clean_torrents(&state);
    }
}
