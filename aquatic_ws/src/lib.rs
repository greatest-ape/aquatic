use aquatic_common::access_list::update_access_list;
#[cfg(feature = "cpu-pinning")]
use aquatic_common::cpu_pinning::{pin_current_if_configured_to, WorkerIndex};
use common::State;
use signal_hook::{consts::SIGUSR1, iterator::Signals};

use std::sync::{atomic::AtomicUsize, Arc};

use crate::{common::create_tls_config, config::Config};
use aquatic_common::privileges::drop_privileges_after_socket_binding;

use glommio::{channels::channel_mesh::MeshBuilder, prelude::*};

pub mod common;
pub mod config;
pub mod workers;

pub const APP_NAME: &str = "aquatic_ws: WebTorrent tracker";

pub const SHARED_IN_CHANNEL_SIZE: usize = 1024;

pub fn run(config: Config) -> ::anyhow::Result<()> {
    let state = State::default();

    update_access_list(&config.access_list, &state.access_list)?;

    let mut signals = Signals::new(::std::iter::once(SIGUSR1))?;

    {
        let config = config.clone();
        let state = state.clone();

        ::std::thread::spawn(move || run_workers(config, state));
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

pub fn run_workers(config: Config, state: State) -> anyhow::Result<()> {
    let num_peers = config.socket_workers + config.request_workers;

    let request_mesh_builder = MeshBuilder::partial(num_peers, SHARED_IN_CHANNEL_SIZE);
    let response_mesh_builder = MeshBuilder::partial(num_peers, SHARED_IN_CHANNEL_SIZE * 16);

    let num_bound_sockets = Arc::new(AtomicUsize::new(0));

    let tls_config = Arc::new(create_tls_config(&config).unwrap());

    let mut executors = Vec::new();

    for i in 0..(config.socket_workers) {
        let config = config.clone();
        let state = state.clone();
        let tls_config = tls_config.clone();
        let request_mesh_builder = request_mesh_builder.clone();
        let response_mesh_builder = response_mesh_builder.clone();
        let num_bound_sockets = num_bound_sockets.clone();

        let builder = LocalExecutorBuilder::default().name("socket");

        let executor = builder.spawn(move || async move {
            #[cfg(feature = "cpu-pinning")]
            pin_current_if_configured_to(
                &config.cpu_pinning,
                config.socket_workers,
                WorkerIndex::SocketWorker(i),
            );

            workers::socket::run_socket_worker(
                config,
                state,
                tls_config,
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

        let builder = LocalExecutorBuilder::default().name("request");

        let executor = builder.spawn(move || async move {
            #[cfg(feature = "cpu-pinning")]
            pin_current_if_configured_to(
                &config.cpu_pinning,
                config.socket_workers,
                WorkerIndex::RequestWorker(i),
            );

            workers::request::run_request_worker(
                config,
                state,
                request_mesh_builder,
                response_mesh_builder,
            )
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

    #[cfg(feature = "cpu-pinning")]
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
