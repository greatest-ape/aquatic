use std::sync::Arc;
use std::thread::Builder;
use std::time::Duration;

use anyhow::Context;
#[cfg(feature = "cpu-pinning")]
use aquatic_common::cpu_pinning::{pin_current_if_configured_to, WorkerIndex};
use histogram::Histogram;
use mio::{Poll, Waker};
use parking_lot::Mutex;
use privdrop::PrivDrop;

pub mod common;
pub mod request;
pub mod socket;

use crate::{common::create_tls_config, config::Config};
use common::*;

pub const APP_NAME: &str = "aquatic_ws: WebTorrent tracker";

const SHARED_IN_CHANNEL_SIZE: usize = 1024;

pub fn run(config: Config, state: State) -> anyhow::Result<()> {
    start_workers(config.clone(), state.clone()).expect("couldn't start workers");

    // TODO: privdrop here instead

    #[cfg(feature = "cpu-pinning")]
    pin_current_if_configured_to(
        &config.cpu_pinning,
        config.socket_workers,
        WorkerIndex::Other,
    );

    loop {
        ::std::thread::sleep(Duration::from_secs(
            config.cleaning.torrent_cleaning_interval,
        ));

        state.torrent_maps.lock().clean(&config, &state.access_list);
    }
}

pub fn start_workers(config: Config, state: State) -> anyhow::Result<()> {
    let tls_config = Arc::new(create_tls_config(&config)?);

    let (in_message_sender, in_message_receiver) = ::crossbeam_channel::bounded(SHARED_IN_CHANNEL_SIZE);

    let mut out_message_senders = Vec::new();
    let mut wakers = Vec::new();

    let socket_worker_statuses: SocketWorkerStatuses = {
        let mut statuses = Vec::new();

        for _ in 0..config.socket_workers {
            statuses.push(None);
        }

        Arc::new(Mutex::new(statuses))
    };

    for i in 0..config.socket_workers {
        let config = config.clone();
        let state = state.clone();
        let socket_worker_statuses = socket_worker_statuses.clone();
        let in_message_sender = in_message_sender.clone();
        let tls_config = tls_config.clone();
        let poll = Poll::new()?;
        let waker = Arc::new(Waker::new(poll.registry(), CHANNEL_TOKEN)?);

        let (out_message_sender, out_message_receiver) = ::crossbeam_channel::bounded(SHARED_IN_CHANNEL_SIZE * 16);

        out_message_senders.push(out_message_sender);
        wakers.push(waker);

        Builder::new()
            .name(format!("socket-{:02}", i + 1))
            .spawn(move || {
                #[cfg(feature = "cpu-pinning")]
                pin_current_if_configured_to(
                    &config.cpu_pinning,
                    config.socket_workers,
                    WorkerIndex::SocketWorker(i),
                );

                socket::run_socket_worker(
                    config,
                    state,
                    i,
                    socket_worker_statuses,
                    poll,
                    in_message_sender,
                    out_message_receiver,
                    tls_config,
                );
            })?;
    }

    // Wait for socket worker statuses. On error from any, quit program.
    // On success from all, drop privileges if corresponding setting is set
    // and continue program.
    loop {
        ::std::thread::sleep(::std::time::Duration::from_millis(10));

        if let Some(statuses) = socket_worker_statuses.try_lock() {
            for opt_status in statuses.iter() {
                if let Some(Err(err)) = opt_status {
                    return Err(::anyhow::anyhow!(err.to_owned()));
                }
            }

            if statuses.iter().all(Option::is_some) {
                if config.privileges.drop_privileges {
                    PrivDrop::default()
                        .chroot(config.privileges.chroot_path.clone())
                        .user(config.privileges.user.clone())
                        .apply()
                        .context("Couldn't drop root privileges")?;
                }

                break;
            }
        }
    }

    let out_message_sender = OutMessageSender::new(out_message_senders);

    for i in 0..config.request_workers {
        let config = config.clone();
        let state = state.clone();
        let in_message_receiver = in_message_receiver.clone();
        let out_message_sender = out_message_sender.clone();
        let wakers = wakers.clone();

        Builder::new()
            .name(format!("request-{:02}", i + 1))
            .spawn(move || {
                #[cfg(feature = "cpu-pinning")]
                pin_current_if_configured_to(
                    &config.cpu_pinning,
                    config.socket_workers,
                    WorkerIndex::RequestWorker(i),
                );

                request::run_request_worker(
                    config,
                    state,
                    in_message_receiver,
                    out_message_sender,
                    wakers,
                );
            })?;
    }

    if config.statistics.interval != 0 {
        let state = state.clone();
        let config = config.clone();

        Builder::new()
            .name("statistics".to_string())
            .spawn(move || {
                #[cfg(feature = "cpu-pinning")]
                pin_current_if_configured_to(
                    &config.cpu_pinning,
                    config.socket_workers,
                    WorkerIndex::Other,
                );

                loop {
                    ::std::thread::sleep(Duration::from_secs(config.statistics.interval));

                    print_statistics(&state);
                }
            })
            .expect("spawn statistics thread");
    }

    Ok(())
}

fn print_statistics(state: &State) {
    let mut peers_per_torrent = Histogram::new();

    {
        let torrents = &mut state.torrent_maps.lock();

        for torrent in torrents.ipv4.values() {
            let num_peers = (torrent.num_seeders + torrent.num_leechers) as u64;

            if let Err(err) = peers_per_torrent.increment(num_peers) {
                eprintln!("error incrementing peers_per_torrent histogram: {}", err)
            }
        }
        for torrent in torrents.ipv6.values() {
            let num_peers = (torrent.num_seeders + torrent.num_leechers) as u64;

            if let Err(err) = peers_per_torrent.increment(num_peers) {
                eprintln!("error incrementing peers_per_torrent histogram: {}", err)
            }
        }
    }

    if peers_per_torrent.entries() != 0 {
        println!(
            "peers per torrent: min: {}, p50: {}, p75: {}, p90: {}, p99: {}, p999: {}, max: {}",
            peers_per_torrent.minimum().unwrap(),
            peers_per_torrent.percentile(50.0).unwrap(),
            peers_per_torrent.percentile(75.0).unwrap(),
            peers_per_torrent.percentile(90.0).unwrap(),
            peers_per_torrent.percentile(99.0).unwrap(),
            peers_per_torrent.percentile(99.9).unwrap(),
            peers_per_torrent.maximum().unwrap(),
        );
    }
}
