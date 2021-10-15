use std::fs::File;
use std::io::Read;
use std::sync::Arc;
use std::thread::Builder;
use std::time::Duration;

use anyhow::Context;
use mio::{Poll, Waker};
use native_tls::{Identity, TlsAcceptor};
use parking_lot::Mutex;
use privdrop::PrivDrop;

pub mod common;
pub mod config;
pub mod handler;
pub mod network;
pub mod tasks;

use common::*;
use config::Config;

pub const APP_NAME: &str = "aquatic_ws: WebTorrent tracker";

pub fn run(config: Config) -> anyhow::Result<()> {
    let state = State::default();

    tasks::update_access_list(&config, &mut state.torrent_maps.lock());

    start_workers(config.clone(), state.clone())?;

    loop {
        ::std::thread::sleep(Duration::from_secs(config.cleaning.interval));

        let mut torrent_maps = state.torrent_maps.lock();

        tasks::update_access_list(&config, &mut torrent_maps);

        torrent_maps.clean(&config);
    }
}

pub fn start_workers(config: Config, state: State) -> anyhow::Result<()> {
    let opt_tls_acceptor = create_tls_acceptor(&config)?;

    let (in_message_sender, in_message_receiver) = ::crossbeam_channel::unbounded();

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
        let socket_worker_statuses = socket_worker_statuses.clone();
        let in_message_sender = in_message_sender.clone();
        let opt_tls_acceptor = opt_tls_acceptor.clone();
        let poll = Poll::new()?;
        let waker = Arc::new(Waker::new(poll.registry(), CHANNEL_TOKEN)?);

        let (out_message_sender, out_message_receiver) = ::crossbeam_channel::unbounded();

        out_message_senders.push(out_message_sender);
        wakers.push(waker);

        Builder::new()
            .name(format!("socket-{:02}", i + 1))
            .spawn(move || {
                network::run_socket_worker(
                    config,
                    i,
                    socket_worker_statuses,
                    poll,
                    in_message_sender,
                    out_message_receiver,
                    opt_tls_acceptor,
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
                handler::run_request_worker(
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
            .spawn(move || loop {
                ::std::thread::sleep(Duration::from_secs(config.statistics.interval));

                tasks::print_statistics(&state);
            })
            .expect("spawn statistics thread");
    }

    Ok(())
}

pub fn create_tls_acceptor(config: &Config) -> anyhow::Result<Option<TlsAcceptor>> {
    if config.network.use_tls {
        let mut identity_bytes = Vec::new();
        let mut file = File::open(&config.network.tls_pkcs12_path)
            .context("Couldn't open pkcs12 identity file")?;

        file.read_to_end(&mut identity_bytes)
            .context("Couldn't read pkcs12 identity file")?;

        let identity = Identity::from_pkcs12(&identity_bytes, &config.network.tls_pkcs12_password)
            .context("Couldn't parse pkcs12 identity file")?;

        let acceptor = TlsAcceptor::new(identity)
            .context("Couldn't create TlsAcceptor from pkcs12 identity")?;

        Ok(Some(acceptor))
    } else {
        Ok(None)
    }
}
