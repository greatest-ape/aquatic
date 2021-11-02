use std::sync::{atomic::AtomicUsize, Arc};
use std::thread::Builder;
use std::time::Duration;

use anyhow::Context;
use aquatic_common::privileges::drop_privileges_after_socket_binding;
use crossbeam_channel::unbounded;

use aquatic_common::access_list::AccessListQuery;
use signal_hook::consts::SIGUSR1;
use signal_hook::iterator::Signals;

use crate::config::Config;

pub mod common;
pub mod handlers;
pub mod network;
pub mod tasks;

use common::State;

pub fn run(config: Config) -> ::anyhow::Result<()> {
    if config.cpu_pinning.active {
        core_affinity::set_for_current(core_affinity::CoreId {
            id: config.cpu_pinning.offset,
        });
    }

    let state = State::default();

    update_access_list(&config, &state)?;

    let mut signals = Signals::new(::std::iter::once(SIGUSR1))?;

    {
        let config = config.clone();
        let state = state.clone();

        ::std::thread::spawn(move || run_inner(config, state));
    }

    for signal in &mut signals {
        match signal {
            SIGUSR1 => {
                let _ = update_access_list(&config, &state);
            }
            _ => unreachable!(),
        }
    }

    Ok(())
}

pub fn run_inner(config: Config, state: State) -> ::anyhow::Result<()> {
    if config.cpu_pinning.active {
        core_affinity::set_for_current(core_affinity::CoreId {
            id: config.cpu_pinning.offset,
        });
    }

    let num_bound_sockets = Arc::new(AtomicUsize::new(0));

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
                if config.cpu_pinning.active {
                    core_affinity::set_for_current(core_affinity::CoreId {
                        id: config.cpu_pinning.offset + 1 + i,
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
                if config.cpu_pinning.active {
                    core_affinity::set_for_current(core_affinity::CoreId {
                        id: config.cpu_pinning.offset + 1 + config.request_workers + i,
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
                if config.cpu_pinning.active {
                    core_affinity::set_for_current(core_affinity::CoreId {
                        id: config.cpu_pinning.offset,
                    });
                }

                loop {
                    ::std::thread::sleep(Duration::from_secs(config.statistics.interval));

                    tasks::gather_and_print_statistics(&state, &config);
                }
            })
            .with_context(|| "spawn statistics worker")?;
    }

    drop_privileges_after_socket_binding(
        &config.privileges,
        num_bound_sockets,
        config.socket_workers,
    )
    .unwrap();

    loop {
        ::std::thread::sleep(Duration::from_secs(config.cleaning.interval));

        state.torrents.lock().clean(&config, &state.access_list);
    }
}

fn update_access_list(config: &Config, state: &State) -> anyhow::Result<()> {
    if config.access_list.mode.is_on() {
        match state.access_list.update(&config.access_list) {
            Ok(()) => {
                ::log::info!("Access list updated")
            }
            Err(err) => {
                ::log::error!("Updating access list failed: {:#}", err);

                return Err(err);
            }
        }
    }

    Ok(())
}
