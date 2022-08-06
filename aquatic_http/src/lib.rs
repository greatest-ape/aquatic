use aquatic_common::{
    access_list::update_access_list,
    cpu_pinning::{
        glommio::{get_worker_placement, set_affinity_for_util_worker},
        WorkerIndex,
    },
    privileges::PrivilegeDropper,
    rustls_config::create_rustls_config,
    PanicSentinelWatcher, ServerStartInstant,
};
use common::State;
use glommio::{channels::channel_mesh::MeshBuilder, prelude::*};
use signal_hook::{
    consts::{SIGTERM, SIGUSR1},
    iterator::Signals,
};
use std::sync::Arc;

use crate::config::Config;

mod common;
pub mod config;
mod workers;

pub const APP_NAME: &str = "aquatic_http: BitTorrent tracker (HTTP over TLS)";
pub const APP_VERSION: &str = env!("CARGO_PKG_VERSION");

const SHARED_CHANNEL_SIZE: usize = 1024;

pub fn run(config: Config) -> ::anyhow::Result<()> {
    let mut signals = Signals::new([SIGUSR1, SIGTERM])?;

    let state = State::default();

    update_access_list(&config.access_list, &state.access_list)?;

    let num_peers = config.socket_workers + config.swarm_workers;

    let request_mesh_builder = MeshBuilder::partial(num_peers, SHARED_CHANNEL_SIZE);

    let (sentinel_watcher, sentinel) = PanicSentinelWatcher::create_with_sentinel();
    let priv_dropper = PrivilegeDropper::new(config.privileges.clone(), config.socket_workers);

    let tls_config = Arc::new(create_rustls_config(
        &config.network.tls_certificate_path,
        &config.network.tls_private_key_path,
    )?);

    let server_start_instant = ServerStartInstant::new();

    let mut executors = Vec::new();

    for i in 0..(config.socket_workers) {
        let sentinel = sentinel.clone();
        let config = config.clone();
        let state = state.clone();
        let tls_config = tls_config.clone();
        let request_mesh_builder = request_mesh_builder.clone();
        let priv_dropper = priv_dropper.clone();

        let placement = get_worker_placement(
            &config.cpu_pinning,
            config.socket_workers,
            config.swarm_workers,
            WorkerIndex::SocketWorker(i),
        )?;
        let builder = LocalExecutorBuilder::new(placement).name("socket");

        let executor = builder
            .spawn(move || async move {
                workers::socket::run_socket_worker(
                    sentinel,
                    config,
                    state,
                    tls_config,
                    request_mesh_builder,
                    priv_dropper,
                    server_start_instant,
                )
                .await
            })
            .map_err(|err| anyhow::anyhow!("Spawning executor failed: {:#}", err))?;

        executors.push(executor);
    }

    for i in 0..(config.swarm_workers) {
        let sentinel = sentinel.clone();
        let config = config.clone();
        let state = state.clone();
        let request_mesh_builder = request_mesh_builder.clone();

        let placement = get_worker_placement(
            &config.cpu_pinning,
            config.socket_workers,
            config.swarm_workers,
            WorkerIndex::SwarmWorker(i),
        )?;
        let builder = LocalExecutorBuilder::new(placement).name("request");

        let executor = builder
            .spawn(move || async move {
                workers::swarm::run_swarm_worker(
                    sentinel,
                    config,
                    state,
                    request_mesh_builder,
                    server_start_instant,
                )
                .await
            })
            .map_err(|err| anyhow::anyhow!("Spawning executor failed: {:#}", err))?;

        executors.push(executor);
    }

    if config.cpu_pinning.active {
        set_affinity_for_util_worker(
            &config.cpu_pinning,
            config.socket_workers,
            config.swarm_workers,
        )?;
    }

    for signal in &mut signals {
        match signal {
            SIGUSR1 => {
                let _ = update_access_list(&config.access_list, &state.access_list);
            }
            SIGTERM => {
                if sentinel_watcher.panic_was_triggered() {
                    return Err(anyhow::anyhow!("worker thread panicked"));
                } else {
                    return Ok(());
                }
            }
            _ => unreachable!(),
        }
    }

    Ok(())
}
