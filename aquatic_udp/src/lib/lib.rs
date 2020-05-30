use std::sync::{Arc, atomic::{AtomicUsize, AtomicBool, Ordering}};
use std::time::Duration;
use std::thread::Builder;

use crossbeam_channel::unbounded;
use privdrop::PrivDrop;

pub mod common;
pub mod config;
pub mod handlers;
pub mod network;
pub mod tasks;

use config::Config;
use common::State;


pub fn run(config: Config) -> ::anyhow::Result<()> {
    let continue_running = Arc::new(AtomicBool::new(true));

    {
        let continue_running = continue_running.clone();

        ::ctrlc::set_handler(move || {
            continue_running.store(false, Ordering::SeqCst)
        }).expect("set ctrlc handler")
    }

    let state = State::new();

    let (request_sender, request_receiver) = unbounded();
    let (response_sender, response_receiver) = unbounded();

    for i in 0..config.request_workers {
        let state = state.clone();
        let config = config.clone();
        let request_receiver = request_receiver.clone();
        let response_sender = response_sender.clone();

        Builder::new().name(format!("request-{:02}", i + 1)).spawn(move ||
            handlers::run_request_worker(
                state,
                config,
                request_receiver,
                response_sender
            )
        ).expect("spawn request worker");
    }

    let num_bound_sockets = Arc::new(AtomicUsize::new(0));

    for i in 0..config.socket_workers {
        let state = state.clone();
        let config = config.clone();
        let request_sender = request_sender.clone();
        let response_receiver = response_receiver.clone();
        let num_bound_sockets = num_bound_sockets.clone();

        Builder::new().name(format!("socket-{:02}", i + 1)).spawn(move ||
            network::run_socket_worker(
                state,
                config,
                i,
                request_sender,
                response_receiver,
                num_bound_sockets,
            )
        ).expect("spawn socket worker");
    }

    if config.statistics.interval != 0 {
        let state = state.clone();
        let config = config.clone();

        Builder::new().name("statistics-collector".to_string()).spawn(move ||
            loop {
                ::std::thread::sleep(Duration::from_secs(
                    config.statistics.interval
                ));

                tasks::gather_and_print_statistics(&state, &config);
            }
        ).expect("spawn statistics thread");
    }

    if config.privileges.drop_privileges {
        loop {
            let sockets = num_bound_sockets.load(Ordering::SeqCst);

            if sockets == config.socket_workers {
                PrivDrop::default()
                    .chroot(config.privileges.chroot_path.clone())
                    .user(config.privileges.user.clone())
                    .apply()
                    .expect("drop privileges");

                break;
            }

            ::std::thread::sleep(Duration::from_millis(10));
        }
    }

    Builder::new().name("cleaning".to_string()).spawn(move ||
        loop {
            ::std::thread::sleep(Duration::from_secs(config.cleaning.interval));

            tasks::clean_connections_and_torrents(&state);
        }
    ).expect("spawn cleaning thread");

    let dur = Duration::from_millis(100);

    while continue_running.load(Ordering::SeqCst){
        ::std::thread::sleep(dur)
    }

    Ok(())
}
