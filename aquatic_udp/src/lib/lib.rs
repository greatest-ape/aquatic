use std::sync::{
    atomic::{AtomicUsize, Ordering},
    Arc,
};
use std::thread::Builder;
use std::time::Duration;

use anyhow::Context;
use aquatic_common::access_list::AccessListMode;
use crossbeam_channel::unbounded;
use privdrop::PrivDrop;

pub mod common;
pub mod config;
pub mod handlers;
pub mod network;
pub mod tasks;

use common::State;
use config::Config;

pub const APP_NAME: &str = "aquatic_udp: UDP BitTorrent tracker";

pub fn run(config: Config) -> ::anyhow::Result<()> {
    let state = State::default();

    match config.access_list.mode {
        AccessListMode::Allow | AccessListMode::Deny => {
            state.access_list.lock().update_from_path(&config.access_list.path)?;
        },
        AccessListMode::Ignore => {},
    }

    let num_bound_sockets = start_workers(config.clone(), state.clone())?;

    if config.privileges.drop_privileges {
        let mut counter = 0usize;

        loop {
            let sockets = num_bound_sockets.load(Ordering::SeqCst);

            if sockets == config.socket_workers {
                PrivDrop::default()
                    .chroot(config.privileges.chroot_path.clone())
                    .user(config.privileges.user.clone())
                    .apply()?;

                break;
            }

            ::std::thread::sleep(Duration::from_millis(10));

            counter += 1;

            if counter == 500 {
                panic!("Sockets didn't bind in time for privilege drop.");
            }
        }
    }

    loop {
        ::std::thread::sleep(Duration::from_secs(config.cleaning.interval));

        tasks::clean_connections_and_torrents(&config, &state);
    }
}

pub fn start_workers(config: Config, state: State) -> ::anyhow::Result<Arc<AtomicUsize>> {
    let (request_sender, request_receiver) = unbounded();
    let (response_sender, response_receiver) = unbounded();

    for i in 0..config.request_workers {
        let state = state.clone();
        let config = config.clone();
        let request_receiver = request_receiver.clone();
        let response_sender = response_sender.clone();

        Builder::new()
            .name(format!("request-{:02}", i + 1))
            .spawn(move || {
                handlers::run_request_worker(state, config, request_receiver, response_sender)
            })
            .with_context(|| "spawn request worker")?;
    }

    let num_bound_sockets = Arc::new(AtomicUsize::new(0));

    for i in 0..config.socket_workers {
        let state = state.clone();
        let config = config.clone();
        let request_sender = request_sender.clone();
        let response_receiver = response_receiver.clone();
        let num_bound_sockets = num_bound_sockets.clone();

        Builder::new()
            .name(format!("socket-{:02}", i + 1))
            .spawn(move || {
                network::run_socket_worker(
                    state,
                    config,
                    i,
                    request_sender,
                    response_receiver,
                    num_bound_sockets,
                )
            })
            .with_context(|| "spawn socket worker")?;
    }

    if config.statistics.interval != 0 {
        let state = state.clone();
        let config = config.clone();

        Builder::new()
            .name("statistics-collector".to_string())
            .spawn(move || loop {
                ::std::thread::sleep(Duration::from_secs(config.statistics.interval));

                tasks::gather_and_print_statistics(&state, &config);
            })
            .with_context(|| "spawn statistics worker")?;
    }

    Ok(num_bound_sockets)
}
