pub mod common;
pub mod config;
pub mod workers;

use std::collections::BTreeMap;
use std::thread::Builder;

use anyhow::Context;
use crossbeam_channel::{bounded, unbounded};
use signal_hook::consts::{SIGTERM, SIGUSR1};
use signal_hook::iterator::Signals;

use aquatic_common::access_list::update_access_list;
#[cfg(feature = "cpu-pinning")]
use aquatic_common::cpu_pinning::{pin_current_if_configured_to, WorkerIndex};
use aquatic_common::privileges::PrivilegeDropper;
use aquatic_common::{PanicSentinelWatcher, ServerStartInstant};

use common::{
    ConnectedRequestSender, ConnectedResponseSender, SocketWorkerIndex, State, SwarmWorkerIndex,
};
use config::Config;
use workers::socket::validator::ConnectionValidator;

pub const APP_NAME: &str = "aquatic_udp: UDP BitTorrent tracker";
pub const APP_VERSION: &str = env!("CARGO_PKG_VERSION");

pub fn run(config: Config) -> ::anyhow::Result<()> {
    let mut signals = Signals::new([SIGUSR1, SIGTERM])?;

    let state = State::new(config.swarm_workers);
    let connection_validator = ConnectionValidator::new(&config)?;
    let (sentinel_watcher, sentinel) = PanicSentinelWatcher::create_with_sentinel();
    let priv_dropper = PrivilegeDropper::new(config.privileges.clone(), config.socket_workers);

    update_access_list(&config.access_list, &state.access_list)?;

    let mut request_senders = Vec::new();
    let mut request_receivers = BTreeMap::new();

    let mut response_senders = Vec::new();
    let mut response_receivers = BTreeMap::new();

    let (statistics_sender, statistics_receiver) = unbounded();

    let server_start_instant = ServerStartInstant::new();

    for i in 0..config.swarm_workers {
        let (request_sender, request_receiver) = if config.worker_channel_size == 0 {
            unbounded()
        } else {
            bounded(config.worker_channel_size)
        };

        request_senders.push(request_sender);
        request_receivers.insert(i, request_receiver);
    }

    for i in 0..config.socket_workers {
        let (response_sender, response_receiver) = if config.worker_channel_size == 0 {
            unbounded()
        } else {
            bounded(config.worker_channel_size)
        };

        response_senders.push(response_sender);
        response_receivers.insert(i, response_receiver);
    }

    for i in 0..config.swarm_workers {
        let sentinel = sentinel.clone();
        let config = config.clone();
        let state = state.clone();
        let request_receiver = request_receivers.remove(&i).unwrap().clone();
        let response_sender = ConnectedResponseSender::new(response_senders.clone());
        let statistics_sender = statistics_sender.clone();

        Builder::new()
            .name(format!("swarm-{:02}", i + 1))
            .spawn(move || {
                #[cfg(feature = "cpu-pinning")]
                pin_current_if_configured_to(
                    &config.cpu_pinning,
                    config.socket_workers,
                    config.swarm_workers,
                    WorkerIndex::SwarmWorker(i),
                );

                workers::swarm::run_swarm_worker(
                    sentinel,
                    config,
                    state,
                    server_start_instant,
                    request_receiver,
                    response_sender,
                    statistics_sender,
                    SwarmWorkerIndex(i),
                )
            })
            .with_context(|| "spawn swarm worker")?;
    }

    for i in 0..config.socket_workers {
        let sentinel = sentinel.clone();
        let state = state.clone();
        let config = config.clone();
        let connection_validator = connection_validator.clone();
        let request_sender =
            ConnectedRequestSender::new(SocketWorkerIndex(i), request_senders.clone());
        let response_receiver = response_receivers.remove(&i).unwrap();
        let priv_dropper = priv_dropper.clone();

        Builder::new()
            .name(format!("socket-{:02}", i + 1))
            .spawn(move || {
                #[cfg(feature = "cpu-pinning")]
                pin_current_if_configured_to(
                    &config.cpu_pinning,
                    config.socket_workers,
                    config.swarm_workers,
                    WorkerIndex::SocketWorker(i),
                );

                workers::socket::SocketWorker::run(
                    sentinel,
                    state,
                    config,
                    connection_validator,
                    server_start_instant,
                    request_sender,
                    response_receiver,
                    priv_dropper,
                );
            })
            .with_context(|| "spawn socket worker")?;
    }

    if config.statistics.active() {
        let sentinel = sentinel.clone();
        let state = state.clone();
        let config = config.clone();

        #[cfg(feature = "prometheus")]
        if config.statistics.run_prometheus_endpoint {
            use metrics_exporter_prometheus::PrometheusBuilder;

            PrometheusBuilder::new()
                .with_http_listener(config.statistics.prometheus_endpoint_address)
                .install()
                .with_context(|| {
                    format!(
                        "Install prometheus endpoint on {}",
                        config.statistics.prometheus_endpoint_address
                    )
                })?;
        }

        Builder::new()
            .name("statistics".into())
            .spawn(move || {
                #[cfg(feature = "cpu-pinning")]
                pin_current_if_configured_to(
                    &config.cpu_pinning,
                    config.socket_workers,
                    config.swarm_workers,
                    WorkerIndex::Util,
                );

                workers::statistics::run_statistics_worker(
                    sentinel,
                    config,
                    state,
                    statistics_receiver,
                );
            })
            .with_context(|| "spawn statistics worker")?;
    }

    #[cfg(feature = "cpu-pinning")]
    pin_current_if_configured_to(
        &config.cpu_pinning,
        config.socket_workers,
        config.swarm_workers,
        WorkerIndex::Util,
    );

    for signal in &mut signals {
        match signal {
            SIGUSR1 => {
                let _ = update_access_list(&config.access_list, &state.access_list);
            }
            SIGTERM => {
                if sentinel_watcher.panic_was_triggered() {
                    return Err(anyhow::anyhow!("worker thread panicked"));
                }

                break;
            }
            _ => unreachable!(),
        }
    }

    Ok(())
}
