use std::time::Duration;

mod common;
mod handler;
mod network;

use common::State;


fn main(){
    let addr = ([127, 0, 0, 1], 3000).into();
    let state = State::new();
    let socket = network::create_socket(addr, 4096 * 8);
    let socket_timeout = Duration::from_millis(1000);

    for i in 1..4 {
        let socket = socket.try_clone().unwrap();
        let state = state.clone();

        ::std::thread::spawn(move || {
            network::run_event_loop(state, socket, i, socket_timeout);
        });
    }

    network::run_event_loop(state, socket, 0, socket_timeout);
}
