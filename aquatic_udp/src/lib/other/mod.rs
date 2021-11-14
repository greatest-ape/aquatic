use std::sync::{atomic::AtomicUsize, Arc};
use std::thread::Builder;
use std::time::Duration;

use anyhow::Context;
#[cfg(feature = "cpu-pinning")]
use aquatic_common::cpu_pinning::{pin_current_if_configured_to, WorkerIndex};
use aquatic_common::privileges::drop_privileges_after_socket_binding;
use crossbeam_channel::unbounded;

use aquatic_common::access_list::update_access_list;
use signal_hook::consts::SIGUSR1;
use signal_hook::iterator::Signals;

use crate::config::Config;

pub mod common;
pub mod handlers;
pub mod network_mio;
pub mod network_uring;
pub mod tasks;

use common::State;

pub fn run(config: Config) -> ::anyhow::Result<()> {
    let state = State::default();

    update_access_list(&config.access_list, &state.access_list)?;

    let mut signals = Signals::new(::std::iter::once(SIGUSR1))?;

    {
        let config = config.clone();
        let state = state.clone();

        ::std::thread::spawn(move || run_inner(config, state));
    }

    #[cfg(feature = "cpu-pinning")]
    pin_current_if_configured_to(
        &config.cpu_pinning,
        config.socket_workers,
        WorkerIndex::Other,
    );

    for signal in &mut signals {
        match signal {
            SIGUSR1 => {
                let _ = update_access_list(&config.access_list, &state.access_list);
            }
            _ => unreachable!(),
        }
    }

    Ok(())
}

pub fn run_inner(config: Config, state: State) -> ::anyhow::Result<()> {
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
                #[cfg(feature = "cpu-pinning")]
                pin_current_if_configured_to(
                    &config.cpu_pinning,
                    config.socket_workers,
                    WorkerIndex::RequestWorker(i),
                );

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
                #[cfg(feature = "cpu-pinning")]
                pin_current_if_configured_to(
                    &config.cpu_pinning,
                    config.socket_workers,
                    WorkerIndex::SocketWorker(i),
                );

                cfg_if::cfg_if!(
                    if #[cfg(feature = "with-io-uring")] {
                        network_uring::run_socket_worker(
                            state,
                            config,
                            request_sender,
                            response_receiver,
                            num_bound_sockets,
                        )
                    } else if #[cfg(feature = "with-mio")] {
                        network_mio::run_socket_worker(
                            state,
                            config,
                            i,
                            request_sender,
                            response_receiver,
                            num_bound_sockets,
                        )
                    }
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
                #[cfg(feature = "cpu-pinning")]
                pin_current_if_configured_to(
                    &config.cpu_pinning,
                    config.socket_workers,
                    WorkerIndex::Other,
                );

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

    #[cfg(feature = "cpu-pinning")]
    pin_current_if_configured_to(
        &config.cpu_pinning,
        config.socket_workers,
        WorkerIndex::Other,
    );

    loop {
        ::std::thread::sleep(Duration::from_secs(
            config.cleaning.torrent_cleaning_interval,
        ));

        state.torrents.lock().clean(&config, &state.access_list);
    }
}
