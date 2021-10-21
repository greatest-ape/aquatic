//! Work-in-progress glommio (io_uring) implementation
//! 
//! * Doesn't support scrape requests
//! * Currently not faster than mio implementation

use std::sync::{atomic::AtomicUsize, Arc};

use glommio::channels::channel_mesh::MeshBuilder;
use glommio::prelude::*;

use crate::config::Config;

mod common;
pub mod handlers;
pub mod network;

pub const SHARED_CHANNEL_SIZE: usize = 4096;

pub fn run(config: Config) -> anyhow::Result<()> {
    if config.core_affinity.set_affinities {
        core_affinity::set_for_current(
            core_affinity::CoreId { id: config.core_affinity.offset }
        );
    }

    let num_peers = config.socket_workers + config.request_workers;

    let request_mesh_builder = MeshBuilder::partial(num_peers, SHARED_CHANNEL_SIZE);
    let response_mesh_builder = MeshBuilder::partial(num_peers, SHARED_CHANNEL_SIZE);

    let num_bound_sockets = Arc::new(AtomicUsize::new(0));

    let mut executors = Vec::new();

    for i in 0..(config.socket_workers) {
        let config = config.clone();
        let request_mesh_builder = request_mesh_builder.clone();
        let response_mesh_builder = response_mesh_builder.clone();
        let num_bound_sockets = num_bound_sockets.clone();

        let mut builder = LocalExecutorBuilder::default();

        if config.core_affinity.set_affinities {
            builder = builder.pin_to_cpu(config.core_affinity.offset + 1 + i);
        }

        let executor = builder.spawn(|| async move {
            network::run_socket_worker(
                config,
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
        let request_mesh_builder = request_mesh_builder.clone();
        let response_mesh_builder = response_mesh_builder.clone();

        let mut builder = LocalExecutorBuilder::default();

        if config.core_affinity.set_affinities {
            builder = builder.pin_to_cpu(config.core_affinity.offset + 1 + config.socket_workers + i);
        }

        let executor = builder.spawn(|| async move {
            handlers::run_request_worker(config, request_mesh_builder, response_mesh_builder).await
        });

        executors.push(executor);
    }

    for executor in executors {
        executor
            .expect("failed to spawn local executor")
            .join()
            .unwrap();
    }

    Ok(())
}
