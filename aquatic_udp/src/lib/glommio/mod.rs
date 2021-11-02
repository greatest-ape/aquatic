use std::sync::{atomic::AtomicUsize, Arc};

use aquatic_common::access_list::AccessListQuery;
use aquatic_common::privileges::drop_privileges_after_socket_binding;
use glommio::channels::channel_mesh::MeshBuilder;
use glommio::prelude::*;
use signal_hook::consts::SIGUSR1;
use signal_hook::iterator::Signals;

use crate::config::Config;

use self::common::State;

mod common;
pub mod handlers;
pub mod network;

pub const SHARED_CHANNEL_SIZE: usize = 4096;

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

pub fn run_inner(config: Config, state: State) -> anyhow::Result<()> {
    if config.cpu_pinning.active {
        core_affinity::set_for_current(core_affinity::CoreId {
            id: config.cpu_pinning.offset,
        });
    }

    let num_peers = config.socket_workers + config.request_workers;

    let request_mesh_builder = MeshBuilder::partial(num_peers, SHARED_CHANNEL_SIZE);
    let response_mesh_builder = MeshBuilder::partial(num_peers, SHARED_CHANNEL_SIZE);

    let num_bound_sockets = Arc::new(AtomicUsize::new(0));

    let mut executors = Vec::new();

    for i in 0..(config.socket_workers) {
        let config = config.clone();
        let state = state.clone();
        let request_mesh_builder = request_mesh_builder.clone();
        let response_mesh_builder = response_mesh_builder.clone();
        let num_bound_sockets = num_bound_sockets.clone();

        let mut builder = LocalExecutorBuilder::default();

        if config.cpu_pinning.active {
            builder = builder.pin_to_cpu(config.cpu_pinning.offset + 1 + i);
        }

        let executor = builder.spawn(|| async move {
            network::run_socket_worker(
                config,
                state,
                request_mesh_builder,
                response_mesh_builder,
                num_bound_sockets,
            )
            .await
        });

        executors.push(executor);
    }

    for i in 0..(config.request_workers) {
        let config = config.clone();
        let state = state.clone();
        let request_mesh_builder = request_mesh_builder.clone();
        let response_mesh_builder = response_mesh_builder.clone();

        let mut builder = LocalExecutorBuilder::default();

        if config.cpu_pinning.active {
            builder = builder.pin_to_cpu(config.cpu_pinning.offset + 1 + config.socket_workers + i);
        }

        let executor = builder.spawn(|| async move {
            handlers::run_request_worker(config, state, request_mesh_builder, response_mesh_builder)
                .await
        });

        executors.push(executor);
    }

    drop_privileges_after_socket_binding(
        &config.privileges,
        num_bound_sockets,
        config.socket_workers,
    )
    .unwrap();

    for executor in executors {
        executor
            .expect("failed to spawn local executor")
            .join()
            .unwrap();
    }

    Ok(())
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
