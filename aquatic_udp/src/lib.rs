pub mod common;
pub mod config;
pub mod workers;

use config::Config;

use std::collections::BTreeMap;
use std::sync::{atomic::AtomicUsize, Arc};
use std::thread::Builder;

use anyhow::Context;
#[cfg(feature = "cpu-pinning")]
use aquatic_common::cpu_pinning::{pin_current_if_configured_to, WorkerIndex};
use aquatic_common::privileges::drop_privileges_after_socket_binding;
use crossbeam_channel::{bounded, unbounded};

use aquatic_common::access_list::update_access_list;
use signal_hook::consts::SIGUSR1;
use signal_hook::iterator::Signals;

use common::{ConnectedRequestSender, ConnectedResponseSender, SocketWorkerIndex, State};

use crate::common::RequestWorkerIndex;

pub const APP_NAME: &str = "aquatic_udp: UDP BitTorrent tracker";
pub const APP_VERSION: &str = env!("CARGO_PKG_VERSION");

pub fn run(config: Config) -> ::anyhow::Result<()> {
    let state = State::new(config.request_workers);

    update_access_list(&config.access_list, &state.access_list)?;

    let mut signals = Signals::new(::std::iter::once(SIGUSR1))?;

    let num_bound_sockets = Arc::new(AtomicUsize::new(0));

    let mut request_senders = Vec::new();
    let mut request_receivers = BTreeMap::new();

    let mut response_senders = Vec::new();
    let mut response_receivers = BTreeMap::new();

    let mut histogram_receivers = Vec::new();

    for i in 0..config.request_workers {
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

    for i in 0..config.request_workers {
        let config = config.clone();
        let state = state.clone();
        let request_receiver = request_receivers.remove(&i).unwrap().clone();
        let response_sender = ConnectedResponseSender::new(response_senders.clone());

        Builder::new()
            .name(format!("request-{:02}", i + 1))
            .spawn(move || {
                #[cfg(feature = "cpu-pinning")]
                pin_current_if_configured_to(
                    &config.cpu_pinning,
                    config.socket_workers,
                    WorkerIndex::RequestWorker(i),
                );

                workers::request::run_request_worker(
                    config,
                    state,
                    request_receiver,
                    response_sender,
                    RequestWorkerIndex(i),
                )
            })
            .with_context(|| "spawn request worker")?;
    }

    for i in 0..config.socket_workers {
        let (histogram_sender, histogram_receiver) = unbounded();

        histogram_receivers.push(histogram_receiver);

        let state = state.clone();
        let config = config.clone();
        let request_sender =
            ConnectedRequestSender::new(SocketWorkerIndex(i), request_senders.clone());
        let response_receiver = response_receivers.remove(&i).unwrap();
        let num_bound_sockets = num_bound_sockets.clone();

        Builder::new()
            .name(format!("socket-{:02}", i + 1))
            .spawn(move || {
                #[cfg(feature = "cpu-pinning")]
                pin_current_if_configured_to(
                    &config.cpu_pinning,
                    config.socket_workers,
                    WorkerIndex::SocketWorker(i),
                );

                workers::socket::run_socket_worker(
                    state,
                    config,
                    i,
                    request_sender,
                    response_receiver,
                    histogram_sender,
                    num_bound_sockets,
                );
            })
            .with_context(|| "spawn socket worker")?;
    }

    if config.statistics.active() {
        let state = state.clone();
        let config = config.clone();

        Builder::new()
            .name("statistics-collector".to_string())
            .spawn(move || {
                #[cfg(feature = "cpu-pinning")]
                pin_current_if_configured_to(
                    &config.cpu_pinning,
                    config.socket_workers,
                    WorkerIndex::Other,
                );

                workers::statistics::run_statistics_worker(config, state, histogram_receivers);
            })
            .with_context(|| "spawn statistics worker")?;
    }

    drop_privileges_after_socket_binding(
        &config.privileges,
        num_bound_sockets,
        config.socket_workers,
    )
    .unwrap();

    #[cfg(feature = "cpu-pinning")]
    pin_current_if_configured_to(
        &config.cpu_pinning,
        config.socket_workers,
        WorkerIndex::Other,
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
}
