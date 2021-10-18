use std::sync::{atomic::AtomicUsize, Arc};

use glommio::channels::channel_mesh::MeshBuilder;

use crate::config::Config;

pub mod handlers;
pub mod network;

pub fn run(config: Config) -> anyhow::Result<()> {
    let num_peers = config.socket_workers + config.request_workers;

    let request_mesh_builder = MeshBuilder::partial(num_peers, 1024);
    let response_mesh_builder = MeshBuilder::partial(num_peers, 1024);

    let num_bound_sockets = Arc::new(AtomicUsize::new(0));

    for _ in 0..(config.socket_workers) {
        network::run_socket_worker(
            config.clone(),
            request_mesh_builder.clone(),
            response_mesh_builder.clone(),
            num_bound_sockets.clone(),
        );
    }

    for _ in 0..(config.request_workers) {
        handlers::run_request_worker(
            config.clone(),
            request_mesh_builder.clone(),
            response_mesh_builder.clone(),
        );
    }

    Ok(())
}
