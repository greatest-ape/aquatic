use std::time::Duration;

pub mod bench_utils;
pub mod common;
pub mod handlers;
pub mod network;
pub mod tasks;

use common::State;


pub fn run(){
    let addr = ([127, 0, 0, 1], 3000).into();
    let state = State::new();
    let socket = network::create_socket(addr, 4096 * 8);
    let socket_timeout = Duration::from_millis(1000);

    for i in 0..4 {
        let socket = socket.try_clone().unwrap();
        let state = state.clone();

        ::std::thread::spawn(move || {
            network::run_event_loop(state, socket, i, socket_timeout);
        });
    }

    loop {
        ::std::thread::sleep(Duration::from_secs(30));

        tasks::clean_connections(&state);
        tasks::clean_torrents(&state);
    }
}
