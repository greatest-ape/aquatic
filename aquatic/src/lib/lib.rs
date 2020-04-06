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

    loop {
        ::std::thread::sleep(Duration::from_secs(30));

        tasks::clean_connections(&state);
        tasks::clean_torrents(&state);
    }
}
