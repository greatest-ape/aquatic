use std::sync::{atomic::AtomicUsize, Arc};

use aquatic_common::access_list::AccessList;
use glommio::channels::channel_mesh::MeshBuilder;
use glommio::prelude::*;

use crate::config::Config;
use crate::drop_privileges_after_socket_binding;

mod common;
pub mod handlers;
pub mod network;

pub const SHARED_CHANNEL_SIZE: usize = 4096;

pub fn run(config: Config) -> anyhow::Result<()> {
    if config.cpu_pinning.active {
        core_affinity::set_for_current(core_affinity::CoreId {
            id: config.cpu_pinning.offset,
        });
    }

    let access_list = if config.access_list.mode.is_on() {
        AccessList::create_from_path(&config.access_list.path).expect("Load access list")
    } else {
        AccessList::default()
    };

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
        let access_list = access_list.clone();

        let mut builder = LocalExecutorBuilder::default();

        if config.cpu_pinning.active {
            builder = builder.pin_to_cpu(config.cpu_pinning.offset + 1 + i);
        }

        let executor = builder.spawn(|| async move {
            network::run_socket_worker(
                config,
                request_mesh_builder,
                response_mesh_builder,
                num_bound_sockets,
                access_list,
            )
            .await
        });

        executors.push(executor);
    }

    for i in 0..(config.request_workers) {
        let config = config.clone();
        let request_mesh_builder = request_mesh_builder.clone();
        let response_mesh_builder = response_mesh_builder.clone();
        let access_list = access_list.clone();

        let mut builder = LocalExecutorBuilder::default();

        if config.cpu_pinning.active {
            builder =
                builder.pin_to_cpu(config.cpu_pinning.offset + 1 + config.socket_workers + i);
        }

        let executor = builder.spawn(|| async move {
            handlers::run_request_worker(
                config,
                request_mesh_builder,
                response_mesh_builder,
                access_list,
            )
            .await
        });

        executors.push(executor);
    }

    drop_privileges_after_socket_binding(&config, num_bound_sockets).unwrap();

    for executor in executors {
        executor
            .expect("failed to spawn local executor")
            .join()
            .unwrap();
    }

    Ok(())
}
