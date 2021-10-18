use std::sync::{atomic::AtomicUsize, Arc};

use glommio::channels::channel_mesh::MeshBuilder;
use glommio::prelude::*;

use crate::config::Config;

pub mod handlers;
pub mod network;

pub fn run(config: Config) -> anyhow::Result<()> {
    let num_peers = config.socket_workers + config.request_workers;

    let request_mesh_builder = MeshBuilder::partial(num_peers, 1024);
    let response_mesh_builder = MeshBuilder::partial(num_peers, 1024);

    let num_bound_sockets = Arc::new(AtomicUsize::new(0));

    let mut executors = Vec::new();

    for _ in 0..(config.socket_workers) {
        let config = config.clone();
        let request_mesh_builder = request_mesh_builder.clone();
        let response_mesh_builder = response_mesh_builder.clone();
        let num_bound_sockets = num_bound_sockets.clone();

        let executor = LocalExecutorBuilder::default().spawn(|| async move {
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

    for _ in 0..(config.request_workers) {
        let config = config.clone();
        let request_mesh_builder = request_mesh_builder.clone();
        let response_mesh_builder = response_mesh_builder.clone();

        let executor = LocalExecutorBuilder::default().spawn(|| async move {
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
