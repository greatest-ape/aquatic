use std::sync::{atomic::AtomicUsize, Arc};

use aquatic_common::access_list::update_access_list;
use aquatic_common::cpu_pinning::{pin_current_if_configured_to, WorkerIndex};
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
    let state = State::default();

    update_access_list(&config.access_list, &state.access_list)?;

    let mut signals = Signals::new(::std::iter::once(SIGUSR1))?;

    {
        let config = config.clone();
        let state = state.clone();

        ::std::thread::spawn(move || run_inner(config, state));
    }

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

pub fn run_inner(config: Config, state: State) -> anyhow::Result<()> {
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

        let executor = LocalExecutorBuilder::default().spawn(move || async move {
            pin_current_if_configured_to(
                &config.cpu_pinning,
                config.socket_workers,
                WorkerIndex::SocketWorker(i),
            );

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

        let executor = LocalExecutorBuilder::default().spawn(move || async move {
            pin_current_if_configured_to(
                &config.cpu_pinning,
                config.socket_workers,
                WorkerIndex::RequestWorker(i),
            );

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

    pin_current_if_configured_to(
        &config.cpu_pinning,
        config.socket_workers,
        WorkerIndex::Other,
    );

    for executor in executors {
        executor
            .expect("failed to spawn local executor")
            .join()
            .unwrap();
    }

    Ok(())
}
