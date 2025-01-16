use anyhow::Context;
use aquatic_common::{
    access_list::update_access_list, privileges::PrivilegeDropper,
    rustls_config::create_rustls_config, ServerStartInstant, WorkerType,
};
use arc_swap::ArcSwap;
use common::State;
use glommio::{channels::channel_mesh::MeshBuilder, prelude::*};
use signal_hook::{consts::SIGUSR1, iterator::Signals};
use std::{
    sync::Arc,
    thread::{sleep, Builder, JoinHandle},
    time::Duration,
};

use crate::config::Config;

mod common;
pub mod config;
mod workers;

pub const APP_NAME: &str = "aquatic_http: HTTP BitTorrent tracker";
pub const APP_VERSION: &str = env!("CARGO_PKG_VERSION");

const SHARED_CHANNEL_SIZE: usize = 1024;

pub fn run(config: Config) -> ::anyhow::Result<()> {
    let mut signals = Signals::new([SIGUSR1])?;

    if !(config.network.use_ipv4 || config.network.use_ipv6) {
        return Result::Err(anyhow::anyhow!(
            "Both use_ipv4 and use_ipv6 can not be set to false"
        ));
    }

    let state = State::default();

    update_access_list(&config.access_list, &state.access_list)?;

    let request_mesh_builder = MeshBuilder::partial(
        config.socket_workers + config.swarm_workers,
        SHARED_CHANNEL_SIZE,
    );

    let num_sockets_per_worker =
        if config.network.use_ipv4 { 1 } else { 0 } + if config.network.use_ipv6 { 1 } else { 0 };

    let priv_dropper = PrivilegeDropper::new(
        config.privileges.clone(),
        config.socket_workers * num_sockets_per_worker,
    );

    let opt_tls_config = if config.network.enable_tls {
        Some(Arc::new(ArcSwap::from_pointee(create_rustls_config(
            &config.network.tls_certificate_path,
            &config.network.tls_private_key_path,
        )?)))
    } else {
        None
    };

    let server_start_instant = ServerStartInstant::new();

    let mut join_handles = Vec::new();

    for i in 0..(config.socket_workers) {
        let config = config.clone();
        let state = state.clone();
        let opt_tls_config = opt_tls_config.clone();
        let request_mesh_builder = request_mesh_builder.clone();

        let mut priv_droppers = Vec::new();

        for _ in 0..num_sockets_per_worker {
            priv_droppers.push(priv_dropper.clone());
        }

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
                        request_mesh_builder,
                        priv_droppers,
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
        let request_mesh_builder = request_mesh_builder.clone();

        let handle = Builder::new()
            .name(format!("swarm-{:02}", i + 1))
            .spawn(move || {
                LocalExecutorBuilder::default()
                    .make()
                    .map_err(|err| anyhow::anyhow!("Spawning executor failed: {:#}", err))?
                    .run(workers::swarm::run_swarm_worker(
                        config,
                        state,
                        request_mesh_builder,
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
                                match create_rustls_config(
                                    &config.network.tls_certificate_path,
                                    &config.network.tls_private_key_path,
                                ) {
                                    Ok(config) => {
                                        tls_config.store(Arc::new(config));

                                        ::log::info!("successfully updated tls config");
                                    }
                                    Err(err) => {
                                        ::log::error!("could not update tls config: {:#}", err)
                                    }
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
