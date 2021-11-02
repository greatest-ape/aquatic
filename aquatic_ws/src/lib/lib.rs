use std::{
    fs::File,
    io::BufReader,
    sync::{atomic::AtomicUsize, Arc},
};

use aquatic_common::{access_list::AccessList, privileges::drop_privileges_after_socket_binding};
use common::TlsConfig;
use glommio::{channels::channel_mesh::MeshBuilder, prelude::*};

use crate::config::Config;

mod common;
pub mod config;
mod handlers;
mod network;

pub const APP_NAME: &str = "aquatic_ws: WebTorrent tracker";

const SHARED_CHANNEL_SIZE: usize = 1024;

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

    let tls_config = Arc::new(create_tls_config(&config).unwrap());

    let mut executors = Vec::new();

    for i in 0..(config.socket_workers) {
        let config = config.clone();
        let tls_config = tls_config.clone();
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
                tls_config,
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
            builder = builder.pin_to_cpu(config.cpu_pinning.offset + 1 + config.socket_workers + i);
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

fn create_tls_config(config: &Config) -> anyhow::Result<TlsConfig> {
    let certs = {
        let f = File::open(&config.network.tls_certificate_path)?;
        let mut f = BufReader::new(f);

        rustls_pemfile::certs(&mut f)?
            .into_iter()
            .map(|bytes| futures_rustls::rustls::Certificate(bytes))
            .collect()
    };

    let private_key = {
        let f = File::open(&config.network.tls_private_key_path)?;
        let mut f = BufReader::new(f);

        rustls_pemfile::pkcs8_private_keys(&mut f)?
            .first()
            .map(|bytes| futures_rustls::rustls::PrivateKey(bytes.clone()))
            .ok_or(anyhow::anyhow!("No private keys in file"))?
    };

    let tls_config = futures_rustls::rustls::ServerConfig::builder()
        .with_safe_defaults()
        .with_no_client_auth()
        .with_single_cert(certs, private_key)?;

    Ok(tls_config)
}
