use std::time::Duration;

use crossbeam_channel::unbounded;

pub mod common;
pub mod config;
pub mod handlers;
pub mod network;
pub mod tasks;

use config::Config;
use common::State;


pub fn run(config: Config){
    let state = State::new();

    let (request_sender, request_receiver) = unbounded();
    let (response_sender, response_receiver) = unbounded();

    for _ in 0..config.request_workers {
        let state = state.clone();
        let config = config.clone();
        let request_receiver = request_receiver.clone();
        let response_sender = response_sender.clone();

        ::std::thread::spawn(move || {
            handlers::run_request_worker(
                state,
                config,
                request_receiver,
                response_sender
            );
        });
    }

    for i in 0..config.socket_workers {
        let state = state.clone();
        let config = config.clone();
        let request_sender = request_sender.clone();
        let response_receiver = response_receiver.clone();

        ::std::thread::spawn(move || {
            network::run_socket_worker(
                state,
                config,
                i,
                request_sender,
                response_receiver
            );
        });
    }

    if config.statistics.interval != 0 {
        let state = state.clone();
        let config = config.clone();

        ::std::thread::spawn(move || {
            loop {
                ::std::thread::sleep(Duration::from_secs(
                    config.statistics.interval
                ));

                tasks::gather_and_print_statistics(&state, &config);
            }
        });
    }

    loop {
        ::std::thread::sleep(Duration::from_secs(config.cleaning.interval));

        tasks::clean_connections_and_torrents(&state, &config);
    }
}
