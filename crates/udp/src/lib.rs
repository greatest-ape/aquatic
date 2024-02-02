pub mod common;
pub mod config;
pub mod workers;

use std::collections::BTreeMap;
use std::fmt::Display;
use std::thread::{sleep, Builder, JoinHandle};
use std::time::Duration;

use anyhow::Context;
use crossbeam_channel::{bounded, unbounded};
use signal_hook::consts::SIGUSR1;
use signal_hook::iterator::Signals;

use aquatic_common::access_list::update_access_list;
#[cfg(feature = "cpu-pinning")]
use aquatic_common::cpu_pinning::{pin_current_if_configured_to, WorkerIndex};
use aquatic_common::privileges::PrivilegeDropper;

use common::{
    ConnectedRequestSender, ConnectedResponseSender, SocketWorkerIndex, State, Statistics,
    SwarmWorkerIndex,
};
use config::Config;
use workers::socket::ConnectionValidator;
use workers::swarm::SwarmWorker;

pub const APP_NAME: &str = "aquatic_udp: UDP BitTorrent tracker";
pub const APP_VERSION: &str = env!("CARGO_PKG_VERSION");

pub fn run(config: Config) -> ::anyhow::Result<()> {
    let mut signals = Signals::new([SIGUSR1])?;

    let state = State::default();
    let statistics = Statistics::new(&config);
    let connection_validator = ConnectionValidator::new(&config)?;
    let priv_dropper = PrivilegeDropper::new(config.privileges.clone(), config.socket_workers);
    let mut join_handles = Vec::new();

    update_access_list(&config.access_list, &state.access_list)?;

    let mut request_senders = Vec::new();
    let mut request_receivers = BTreeMap::new();

    let mut response_senders = Vec::new();
    let mut response_receivers = BTreeMap::new();

    let (statistics_sender, statistics_receiver) = unbounded();

    for i in 0..config.swarm_workers {
        let (request_sender, request_receiver) = bounded(config.worker_channel_size);

        request_senders.push(request_sender);
        request_receivers.insert(i, request_receiver);
    }

    for i in 0..config.socket_workers {
        let (response_sender, response_receiver) = bounded(config.worker_channel_size);

        response_senders.push(response_sender);
        response_receivers.insert(i, response_receiver);
    }

    for i in 0..config.swarm_workers {
        let config = config.clone();
        let state = state.clone();
        let request_receiver = request_receivers.remove(&i).unwrap().clone();
        let response_sender = ConnectedResponseSender::new(response_senders.clone());
        let statistics_sender = statistics_sender.clone();
        let statistics = statistics.swarm[i].clone();

        let handle = Builder::new()
            .name(format!("swarm-{:02}", i + 1))
            .spawn(move || {
                #[cfg(feature = "cpu-pinning")]
                pin_current_if_configured_to(
                    &config.cpu_pinning,
                    config.socket_workers,
                    config.swarm_workers,
                    WorkerIndex::SwarmWorker(i),
                );

                let mut worker = SwarmWorker {
                    config,
                    state,
                    statistics,
                    request_receiver,
                    response_sender,
                    statistics_sender,
                    worker_index: SwarmWorkerIndex(i),
                };

                worker.run()
            })
            .with_context(|| "spawn swarm worker")?;

        join_handles.push((WorkerType::Swarm(i), handle));
    }

    for i in 0..config.socket_workers {
        let state = state.clone();
        let config = config.clone();
        let connection_validator = connection_validator.clone();
        let request_sender =
            ConnectedRequestSender::new(SocketWorkerIndex(i), request_senders.clone());
        let response_receiver = response_receivers.remove(&i).unwrap();
        let priv_dropper = priv_dropper.clone();
        let statistics = statistics.socket[i].clone();

        let handle = Builder::new()
            .name(format!("socket-{:02}", i + 1))
            .spawn(move || {
                #[cfg(feature = "cpu-pinning")]
                pin_current_if_configured_to(
                    &config.cpu_pinning,
                    config.socket_workers,
                    config.swarm_workers,
                    WorkerIndex::SocketWorker(i),
                );

                workers::socket::run_socket_worker(
                    config,
                    state,
                    statistics,
                    connection_validator,
                    request_sender,
                    response_receiver,
                    priv_dropper,
                )
            })
            .with_context(|| "spawn socket worker")?;

        join_handles.push((WorkerType::Socket(i), handle));
    }

    if config.statistics.active() {
        let state = state.clone();
        let config = config.clone();

        let handle = Builder::new()
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
                    config,
                    state,
                    statistics,
                    statistics_receiver,
                )
            })
            .with_context(|| "spawn statistics worker")?;

        join_handles.push((WorkerType::Statistics, handle));
    }

    #[cfg(feature = "prometheus")]
    if config.statistics.active() && config.statistics.run_prometheus_endpoint {
        let config = config.clone();

        let handle = Builder::new()
            .name("prometheus".into())
            .spawn(move || {
                #[cfg(feature = "cpu-pinning")]
                pin_current_if_configured_to(
                    &config.cpu_pinning,
                    config.socket_workers,
                    config.swarm_workers,
                    WorkerIndex::Util,
                );

                use metrics_exporter_prometheus::PrometheusBuilder;
                use metrics_util::MetricKindMask;

                let rt = ::tokio::runtime::Builder::new_current_thread()
                    .enable_all()
                    .build()
                    .context("build prometheus tokio runtime")?;

                rt.block_on(async {
                    let (recorder, exporter) = PrometheusBuilder::new()
                        .idle_timeout(
                            MetricKindMask::ALL,
                            Some(Duration::from_secs(config.statistics.interval * 2)),
                        )
                        .with_http_listener(config.statistics.prometheus_endpoint_address)
                        .build()
                        .context("build prometheus recorder and exporter")?;

                    let recorder_handle = recorder.handle();

                    ::metrics::set_global_recorder(recorder)
                        .context("set global metrics recorder")?;

                    ::tokio::spawn(async move {
                        let mut interval = ::tokio::time::interval(Duration::from_secs(5));

                        loop {
                            interval.tick().await;

                            // Periodically render metrics to make sure
                            // idles are cleaned up
                            recorder_handle.render();
                        }
                    });

                    exporter.await.context("run prometheus exporter")
                })
            })
            .with_context(|| "spawn prometheus exporter worker")?;

        join_handles.push((WorkerType::Prometheus, handle));
    }

    // Spawn signal handler thread
    {
        let config = config.clone();

        let handle: JoinHandle<anyhow::Result<()>> = Builder::new()
            .name("signals".into())
            .spawn(move || {
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
                        _ => unreachable!(),
                    }
                }

                Ok(())
            })
            .context("spawn signal worker")?;

        join_handles.push((WorkerType::Signals, handle));
    }

    #[cfg(feature = "cpu-pinning")]
    pin_current_if_configured_to(
        &config.cpu_pinning,
        config.socket_workers,
        config.swarm_workers,
        WorkerIndex::Util,
    );

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

enum WorkerType {
    Swarm(usize),
    Socket(usize),
    Statistics,
    Signals,
    #[cfg(feature = "prometheus")]
    Prometheus,
}

impl Display for WorkerType {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Swarm(index) => f.write_fmt(format_args!("Swarm worker {}", index + 1)),
            Self::Socket(index) => f.write_fmt(format_args!("Socket worker {}", index + 1)),
            Self::Statistics => f.write_str("Statistics worker"),
            Self::Signals => f.write_str("Signals worker"),
            #[cfg(feature = "prometheus")]
            Self::Prometheus => f.write_str("Prometheus worker"),
        }
    }
}
