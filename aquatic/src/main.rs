use std::time::Duration;

mod handler;
mod network;
mod types;

use types::State;


fn main(){
    let addr = ([127, 0, 0, 1], 3000).into();
    let socket = network::create_socket(addr, 4096 * 8);
    let state = State::new();

    for i in 1..4 {
        let socket = socket.try_clone().unwrap();
        let state = state.clone();

        ::std::thread::spawn(move || {
            network::run_event_loop(state, socket, i, 4096, Duration::from_millis(1000));
        });
    }

    network::run_event_loop(state, socket, 0, 4096, Duration::from_millis(1000));
}
