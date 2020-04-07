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

                let requests_per_second = requests_received / interval as f64;
                let responses_per_second: f64 = responses_sent / interval as f64;

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
            }
        });
    }

    loop {
        ::std::thread::sleep(Duration::from_secs(30));

        tasks::clean_connections(&state);
        tasks::clean_torrents(&state);
    }
}
