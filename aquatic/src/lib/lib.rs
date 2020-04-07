use std::sync::atomic::Ordering;
use std::time::Duration;

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
    let socket = network::create_socket(&config);

    for i in 0..4 {
        let socket = socket.try_clone().unwrap();
        let state = state.clone();
        let config = config.clone();

        ::std::thread::spawn(move || {
            network::run_event_loop(state, config, socket, i);
        });
    }

    {
        let state = state.clone();

        ::std::thread::spawn(move || {
            let interval = config.statistics_interval;

            loop {
                ::std::thread::sleep(Duration::from_secs(interval));

                let requests_per_second: f64 = state.statistics.requests_received
                    .fetch_and(0, Ordering::SeqCst) as f64 / interval as f64;
                let responses_per_second: f64 = state.statistics.responses_sent
                    .fetch_and(0, Ordering::SeqCst) as f64 / interval as f64;

                println!(
                    "stats: {} requests/second, {} responses/second",
                    requests_per_second,
                    responses_per_second
                );
            }
        });
    }

    loop {
        ::std::thread::sleep(Duration::from_secs(30));

        tasks::clean_connections(&state);
        tasks::clean_torrents(&state);
    }
}
