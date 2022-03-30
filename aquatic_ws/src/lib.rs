pub mod common;
pub mod config;
pub mod workers;

use std::fs::File;
use std::io::BufReader;
use std::sync::{atomic::AtomicUsize, Arc};

use aquatic_common::cpu_pinning::glommio::{get_worker_placement, set_affinity_for_util_worker};
use aquatic_common::cpu_pinning::WorkerIndex;
use glommio::{channels::channel_mesh::MeshBuilder, prelude::*};
use signal_hook::{consts::SIGUSR1, iterator::Signals};

use aquatic_common::access_list::update_access_list;
use aquatic_common::privileges::drop_privileges_after_socket_binding;

use common::*;
use config::Config;

pub const APP_NAME: &str = "aquatic_ws: WebTorrent tracker";
pub const APP_VERSION: &str = env!("CARGO_PKG_VERSION");

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

    if config.cpu_pinning.active {
        set_affinity_for_util_worker(
            &config.cpu_pinning,
            config.socket_workers,
            config.request_workers,
        )?;
    }

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

fn run_workers(config: Config, state: State) -> anyhow::Result<()> {
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

        let placement = get_worker_placement(
            &config.cpu_pinning,
            config.socket_workers,
            config.request_workers,
            WorkerIndex::SocketWorker(i),
        )?;
        let builder = LocalExecutorBuilder::new(placement).name("socket");

        let executor = builder.spawn(move || async move {
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

        let placement = get_worker_placement(
            &config.cpu_pinning,
            config.socket_workers,
            config.request_workers,
            WorkerIndex::RequestWorker(i),
        )?;
        let builder = LocalExecutorBuilder::new(placement).name("request");

        let executor = builder.spawn(move || async move {
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

    if config.cpu_pinning.active {
        set_affinity_for_util_worker(
            &config.cpu_pinning,
            config.socket_workers,
            config.request_workers,
        )?;
    }

    for executor in executors {
        executor
            .expect("failed to spawn local executor")
            .join()
            .unwrap();
    }

    Ok(())
}

fn create_tls_config(config: &Config) -> anyhow::Result<rustls::ServerConfig> {
    let certs = {
        let f = File::open(&config.network.tls_certificate_path)?;
        let mut f = BufReader::new(f);

        rustls_pemfile::certs(&mut f)?
            .into_iter()
            .map(|bytes| rustls::Certificate(bytes))
            .collect()
    };

    let private_key = {
        let f = File::open(&config.network.tls_private_key_path)?;
        let mut f = BufReader::new(f);

        rustls_pemfile::pkcs8_private_keys(&mut f)?
            .first()
            .map(|bytes| rustls::PrivateKey(bytes.clone()))
            .ok_or(anyhow::anyhow!("No private keys in file"))?
    };

    let tls_config = rustls::ServerConfig::builder()
        .with_safe_defaults()
        .with_no_client_auth()
        .with_single_cert(certs, private_key)?;

    Ok(tls_config)
}
