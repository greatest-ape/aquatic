pub mod common;
pub mod config;
pub mod workers;

use std::sync::Arc;
use std::time::Duration;

use anyhow::Context;
use aquatic_common::cpu_pinning::glommio::{get_worker_placement, set_affinity_for_util_worker};
use aquatic_common::cpu_pinning::WorkerIndex;
use aquatic_common::rustls_config::create_rustls_config;
use aquatic_common::{PanicSentinelWatcher, ServerStartInstant};
use arc_swap::ArcSwap;
use glommio::{channels::channel_mesh::MeshBuilder, prelude::*};
use signal_hook::{
    consts::{SIGTERM, SIGUSR1},
    iterator::Signals,
};

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

    let mut signals = Signals::new([SIGUSR1, SIGTERM])?;

    #[cfg(feature = "prometheus")]
    if config.metrics.run_prometheus_endpoint {
        use metrics_exporter_prometheus::PrometheusBuilder;

        let idle_timeout = config
            .cleaning
            .connection_cleaning_interval
            .max(config.cleaning.torrent_cleaning_interval)
            .max(config.metrics.torrent_count_update_interval)
            * 2;

        PrometheusBuilder::new()
            .idle_timeout(
                metrics_util::MetricKindMask::GAUGE,
                Some(Duration::from_secs(idle_timeout)),
            )
            .with_http_listener(config.metrics.prometheus_endpoint_address)
            .install()
            .with_context(|| {
                format!(
                    "Install prometheus endpoint on {}",
                    config.metrics.prometheus_endpoint_address
                )
            })?;
    }

    let state = State::default();

    update_access_list(&config.access_list, &state.access_list)?;

    let num_peers = config.socket_workers + config.swarm_workers;

    let request_mesh_builder = MeshBuilder::partial(num_peers, SHARED_IN_CHANNEL_SIZE);
    let response_mesh_builder = MeshBuilder::partial(num_peers, SHARED_IN_CHANNEL_SIZE * 16);
    let control_mesh_builder = MeshBuilder::partial(num_peers, SHARED_IN_CHANNEL_SIZE * 16);

    let (sentinel_watcher, sentinel) = PanicSentinelWatcher::create_with_sentinel();
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

    let mut executors = Vec::new();

    for i in 0..(config.socket_workers) {
        let sentinel = sentinel.clone();
        let config = config.clone();
        let state = state.clone();
        let opt_tls_config = opt_tls_config.clone();
        let control_mesh_builder = control_mesh_builder.clone();
        let request_mesh_builder = request_mesh_builder.clone();
        let response_mesh_builder = response_mesh_builder.clone();
        let priv_dropper = priv_dropper.clone();

        let placement = get_worker_placement(
            &config.cpu_pinning,
            config.socket_workers,
            config.swarm_workers,
            WorkerIndex::SocketWorker(i),
        )?;
        let builder = LocalExecutorBuilder::new(placement).name(&format!("socket-{:02}", i + 1));

        let executor = builder
            .spawn(move || async move {
                workers::socket::run_socket_worker(
                    sentinel,
                    config,
                    state,
                    opt_tls_config,
                    control_mesh_builder,
                    request_mesh_builder,
                    response_mesh_builder,
                    priv_dropper,
                    server_start_instant,
                    i,
                )
                .await
            })
            .map_err(|err| anyhow::anyhow!("Spawning executor failed: {:#}", err))?;

        executors.push(executor);
    }

    ::log::info!("spawned socket workers");

    for i in 0..(config.swarm_workers) {
        let sentinel = sentinel.clone();
        let config = config.clone();
        let state = state.clone();
        let control_mesh_builder = control_mesh_builder.clone();
        let request_mesh_builder = request_mesh_builder.clone();
        let response_mesh_builder = response_mesh_builder.clone();

        let placement = get_worker_placement(
            &config.cpu_pinning,
            config.socket_workers,
            config.swarm_workers,
            WorkerIndex::SwarmWorker(i),
        )?;
        let builder = LocalExecutorBuilder::new(placement).name(&format!("swarm-{:02}", i + 1));

        let executor = builder
            .spawn(move || async move {
                workers::swarm::run_swarm_worker(
                    sentinel,
                    config,
                    state,
                    control_mesh_builder,
                    request_mesh_builder,
                    response_mesh_builder,
                    server_start_instant,
                    i,
                )
                .await
            })
            .map_err(|err| anyhow::anyhow!("Spawning executor failed: {:#}", err))?;

        executors.push(executor);
    }

    ::log::info!("spawned swarm workers");

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
