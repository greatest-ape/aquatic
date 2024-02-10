pub mod common;
pub mod config;
pub mod swarm;
pub mod workers;

use std::thread::{sleep, Builder, JoinHandle};
use std::time::Duration;

use anyhow::Context;
use aquatic_common::WorkerType;
use crossbeam_channel::unbounded;
use signal_hook::consts::SIGUSR1;
use signal_hook::iterator::Signals;

use aquatic_common::access_list::update_access_list;
#[cfg(feature = "cpu-pinning")]
use aquatic_common::cpu_pinning::{pin_current_if_configured_to, WorkerIndex};
use aquatic_common::privileges::PrivilegeDropper;

use common::{State, Statistics};
use config::Config;
use workers::socket::ConnectionValidator;

pub const APP_NAME: &str = "aquatic_udp: UDP BitTorrent tracker";
pub const APP_VERSION: &str = env!("CARGO_PKG_VERSION");

pub fn run(config: Config) -> ::anyhow::Result<()> {
    let mut signals = Signals::new([SIGUSR1])?;

    let state = State::new(&config);
    let statistics = Statistics::new(&config);
    let connection_validator = ConnectionValidator::new(&config)?;
    let priv_dropper = PrivilegeDropper::new(config.privileges.clone(), config.socket_workers);

    let mut join_handles = Vec::new();

    update_access_list(&config.access_list, &state.access_list)?;

    let (statistics_sender, statistics_receiver) = unbounded();

    for i in 0..config.socket_workers {
        let state = state.clone();
        let config = config.clone();
        let connection_validator = connection_validator.clone();
        let priv_dropper = priv_dropper.clone();
        let statistics = statistics.socket[i].clone();
        let statistics_sender = statistics_sender.clone();

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
                    statistics_sender,
                    connection_validator,
                    priv_dropper,
                )
            })
            .with_context(|| "spawn socket worker")?;

        join_handles.push((WorkerType::Socket(i), handle));
    }

    {
        let state = state.clone();
        let config = config.clone();
        let statistics = statistics.swarm.clone();
        let statistics_sender = statistics_sender.clone();

        let handle = Builder::new().name("cleaning".into()).spawn(move || loop {
            sleep(Duration::from_secs(
                config.cleaning.torrent_cleaning_interval,
            ));

            state.torrent_maps.clean_and_update_statistics(
                &config,
                &statistics,
                &statistics_sender,
                &state.access_list,
                state.server_start_instant,
            );
        })?;

        join_handles.push((WorkerType::Cleaning, handle));
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
        let handle = aquatic_common::spawn_prometheus_endpoint(
            config.statistics.prometheus_endpoint_address,
            Some(Duration::from_secs(
                config.cleaning.torrent_cleaning_interval * 2,
            )),
            None,
        )?;

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
