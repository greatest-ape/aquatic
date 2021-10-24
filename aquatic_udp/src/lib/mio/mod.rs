use std::thread::Builder;
use std::time::Duration;
use std::{
    ops::Deref,
    sync::{atomic::AtomicUsize, Arc},
};

use anyhow::Context;
use crossbeam_channel::unbounded;

pub mod common;
pub mod handlers;
pub mod network;
pub mod tasks;

use aquatic_common::access_list::{AccessListArcSwap, AccessListMode, AccessListQuery};

use crate::config::Config;
use crate::drop_privileges_after_socket_binding;

use common::State;

pub fn run(config: Config) -> ::anyhow::Result<()> {
    if config.core_affinity.set_affinities {
        core_affinity::set_for_current(core_affinity::CoreId {
            id: config.core_affinity.offset,
        });
    }

    let state = State::default();

    update_access_list(&config, &state.access_list);

    let num_bound_sockets = Arc::new(AtomicUsize::new(0));

    start_workers(config.clone(), state.clone(), num_bound_sockets.clone())?;

    drop_privileges_after_socket_binding(&config, num_bound_sockets).unwrap();

    loop {
        ::std::thread::sleep(Duration::from_secs(config.cleaning.interval));

        update_access_list(&config, &state.access_list);

        state
            .torrents
            .lock()
            .clean(&config, state.access_list.load_full().deref());
    }
}

pub fn start_workers(
    config: Config,
    state: State,
    num_bound_sockets: Arc<AtomicUsize>,
) -> ::anyhow::Result<()> {
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
                if config.core_affinity.set_affinities {
                    core_affinity::set_for_current(core_affinity::CoreId {
                        id: config.core_affinity.offset + 1 + i,
                    });
                }

                handlers::run_request_worker(state, config, request_receiver, response_sender)
            })
            .with_context(|| "spawn request worker")?;
    }

    for i in 0..config.socket_workers {
        let state = state.clone();
        let config = config.clone();
        let request_sender = request_sender.clone();
        let response_receiver = response_receiver.clone();
        let num_bound_sockets = num_bound_sockets.clone();

        Builder::new()
            .name(format!("socket-{:02}", i + 1))
            .spawn(move || {
                if config.core_affinity.set_affinities {
                    core_affinity::set_for_current(core_affinity::CoreId {
                        id: config.core_affinity.offset + 1 + config.request_workers + i,
                    });
                }

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
            .spawn(move || {
                if config.core_affinity.set_affinities {
                    core_affinity::set_for_current(core_affinity::CoreId {
                        id: config.core_affinity.offset,
                    });
                }

                loop {
                    ::std::thread::sleep(Duration::from_secs(config.statistics.interval));

                    tasks::gather_and_print_statistics(&state, &config);
                }
            })
            .with_context(|| "spawn statistics worker")?;
    }

    Ok(())
}

pub fn update_access_list(config: &Config, access_list: &Arc<AccessListArcSwap>) {
    match config.access_list.mode {
        AccessListMode::White | AccessListMode::Black => {
            if let Err(err) = access_list.update_from_path(&config.access_list.path) {
                ::log::error!("Update access list from path: {:?}", err);
            }
        }
        AccessListMode::Off => {}
    }
}
