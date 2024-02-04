pub mod common;
pub mod config;
pub mod workers;

use std::sync::Arc;
use std::thread::{sleep, Builder, JoinHandle};
use std::time::Duration;

use anyhow::Context;
use aquatic_common::rustls_config::create_rustls_config;
use aquatic_common::{ServerStartInstant, WorkerType};
use arc_swap::ArcSwap;
use glommio::{channels::channel_mesh::MeshBuilder, prelude::*};
use signal_hook::{consts::SIGUSR1, iterator::Signals};

use aquatic_common::access_list::update_access_list;
use aquatic_common::privileges::PrivilegeDropper;

use common::*;
use config::Config;

pub const APP_NAME: &str = "aquatic_ws: WebTorrent tracker";
pub const APP_VERSION: &str = env!("CARGO_PKG_VERSION");

pub const SHARED_IN_CHANNEL_SIZE: usize = 1024;

pub fn run(config: Config) -> ::anyhow::Result<()> {
    if config.network.enable_tls && config.network.enable_http_health_checks {
        return Err(anyhow::anyhow!(
            "configuration: network.enable_tls and network.enable_http_health_check can't both be set to true"
        ));
    }

    let mut signals = Signals::new([SIGUSR1])?;

    let state = State::default();

    update_access_list(&config.access_list, &state.access_list)?;

    let num_mesh_peers = config.socket_workers + config.swarm_workers;

    let request_mesh_builder = MeshBuilder::partial(num_mesh_peers, SHARED_IN_CHANNEL_SIZE);
    let response_mesh_builder = MeshBuilder::partial(num_mesh_peers, SHARED_IN_CHANNEL_SIZE * 16);
    let control_mesh_builder = MeshBuilder::partial(num_mesh_peers, SHARED_IN_CHANNEL_SIZE * 16);

    let priv_dropper = PrivilegeDropper::new(config.privileges.clone(), config.socket_workers);

    let opt_tls_config = if config.network.enable_tls {
        Some(Arc::new(ArcSwap::from_pointee(
            create_rustls_config(
                &config.network.tls_certificate_path,
                &config.network.tls_private_key_path,
            )
            .with_context(|| "create rustls config")?,
        )))
    } else {
        None
    };
    let mut opt_tls_cert_data = if config.network.enable_tls {
        Some(
            ::std::fs::read(&config.network.tls_certificate_path)
                .with_context(|| "open tls certificate file")?,
        )
    } else {
        None
    };

    let server_start_instant = ServerStartInstant::new();

    let mut join_handles = Vec::new();

    for i in 0..(config.socket_workers) {
        let config = config.clone();
        let state = state.clone();
        let opt_tls_config = opt_tls_config.clone();
        let control_mesh_builder = control_mesh_builder.clone();
        let request_mesh_builder = request_mesh_builder.clone();
        let response_mesh_builder = response_mesh_builder.clone();
        let priv_dropper = priv_dropper.clone();

        let handle = Builder::new()
            .name(format!("socket-{:02}", i + 1))
            .spawn(move || {
                LocalExecutorBuilder::default()
                    .make()
                    .map_err(|err| anyhow::anyhow!("Spawning executor failed: {:#}", err))?
                    .run(workers::socket::run_socket_worker(
                        config,
                        state,
                        opt_tls_config,
                        control_mesh_builder,
                        request_mesh_builder,
                        response_mesh_builder,
                        priv_dropper,
                        server_start_instant,
                        i,
                    ))
            })
            .context("spawn socket worker")?;

        join_handles.push((WorkerType::Socket(i), handle));
    }

    for i in 0..(config.swarm_workers) {
        let config = config.clone();
        let state = state.clone();
        let control_mesh_builder = control_mesh_builder.clone();
        let request_mesh_builder = request_mesh_builder.clone();
        let response_mesh_builder = response_mesh_builder.clone();

        let handle = Builder::new()
            .name(format!("swarm-{:02}", i + 1))
            .spawn(move || {
                LocalExecutorBuilder::default()
                    .make()
                    .map_err(|err| anyhow::anyhow!("Spawning executor failed: {:#}", err))?
                    .run(workers::swarm::run_swarm_worker(
                        config,
                        state,
                        control_mesh_builder,
                        request_mesh_builder,
                        response_mesh_builder,
                        server_start_instant,
                        i,
                    ))
            })
            .context("spawn swarm worker")?;

        join_handles.push((WorkerType::Swarm(i), handle));
    }

    #[cfg(feature = "prometheus")]
    if config.metrics.run_prometheus_endpoint {
        let idle_timeout = config
            .cleaning
            .connection_cleaning_interval
            .max(config.cleaning.torrent_cleaning_interval)
            .max(config.metrics.torrent_count_update_interval)
            * 2;

        let handle = aquatic_common::spawn_prometheus_endpoint(
            config.metrics.prometheus_endpoint_address,
            Some(Duration::from_secs(idle_timeout)),
            Some(metrics_util::MetricKindMask::GAUGE),
        )?;

        join_handles.push((WorkerType::Prometheus, handle));
    }

    // Spawn signal handler thread
    {
        let handle: JoinHandle<anyhow::Result<()>> = Builder::new()
            .name("signals".into())
            .spawn(move || {
                for signal in &mut signals {
                    match signal {
                        SIGUSR1 => {
                            let _ = update_access_list(&config.access_list, &state.access_list);

                            if let Some(tls_config) = opt_tls_config.as_ref() {
                                match ::std::fs::read(&config.network.tls_certificate_path) {
                                    Ok(data) if &data == opt_tls_cert_data.as_ref().unwrap() => {
                                        ::log::info!("skipping tls config update: certificate identical to currently loaded");
                                    }
                                    Ok(data) => {
                                        match create_rustls_config(
                                            &config.network.tls_certificate_path,
                                            &config.network.tls_private_key_path,
                                        ) {
                                            Ok(config) => {
                                                tls_config.store(Arc::new(config));
                                                opt_tls_cert_data = Some(data);

                                                ::log::info!("successfully updated tls config");
                                            }
                                            Err(err) => ::log::error!("could not update tls config: {:#}", err),
                                        }
                                    }
                                    Err(err) => ::log::error!("couldn't read tls certificate file: {:#}", err),
                                }
                            }
                        }
                        _ => unreachable!(),
                    }
                }

                Ok(())
            })
            .context("spawn signal worker")?;

        join_handles.push((WorkerType::Signals, handle));
    }

    loop {
        for (i, (_, handle)) in join_handles.iter().enumerate() {
            if handle.is_finished() {
                let (worker_type, handle) = join_handles.remove(i);

                match handle.join() {
                    Ok(Ok(())) => {
                        return Err(anyhow::anyhow!("{} stopped", worker_type));
                    }
                    Ok(Err(err)) => {
                        return Err(err.context(format!("{} stopped", worker_type)));
                    }
                    Err(_) => {
                        return Err(anyhow::anyhow!("{} panicked", worker_type));
                    }
                }
            }
        }

        sleep(Duration::from_secs(5));
    }
}
