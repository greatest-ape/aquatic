use std::time::Duration;
use std::sync::Arc;
use std::thread::Builder;

use anyhow::Context;
use mio::{Poll, Waker};
use parking_lot::Mutex;
use privdrop::PrivDrop;

pub mod common;
pub mod config;
pub mod handler;
pub mod network;
pub mod tasks;

use common::*;
use config::Config;
use network::utils::create_tls_acceptor;


pub const APP_NAME: &str = "aquatic_http: HTTP/TLS BitTorrent tracker";


pub fn run(config: Config) -> anyhow::Result<()> {
    let state = State::default();

    start_workers(config.clone(), state.clone())?;

    loop {
        ::std::thread::sleep(Duration::from_secs(config.cleaning.interval));

        tasks::clean_torrents(&state);
    }
}


pub fn start_workers(config: Config, state: State) -> anyhow::Result<()> {
    let opt_tls_acceptor = create_tls_acceptor(&config.network.tls)?;

    let (request_channel_sender, request_channel_receiver) = ::crossbeam_channel::unbounded();

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
        let request_channel_sender = request_channel_sender.clone();
        let opt_tls_acceptor = opt_tls_acceptor.clone();
        let poll = Poll::new().expect("create poll");
        let waker = Arc::new(Waker::new(poll.registry(), CHANNEL_TOKEN).expect("create waker"));

        let (response_channel_sender, response_channel_receiver) = ::crossbeam_channel::unbounded();

        out_message_senders.push(response_channel_sender);
        wakers.push(waker);

        Builder::new().name(format!("socket-{:02}", i + 1)).spawn(move || {
            network::run_socket_worker(
                config,
                i,
                socket_worker_statuses,
                request_channel_sender,
                response_channel_receiver,
                opt_tls_acceptor,
                poll
            );
        })?;
    }

    // Wait for socket worker statuses. On error from any, quit program.
    // On success from all, drop privileges if corresponding setting is set
    // and continue program.
    loop {
        ::std::thread::sleep(::std::time::Duration::from_millis(10));

        if let Some(statuses) = socket_worker_statuses.try_lock(){
            for opt_status in statuses.iter(){
                if let Some(Err(err))  = opt_status {
                    return Err(::anyhow::anyhow!(err.to_owned()));
                }
            }

            if statuses.iter().all(Option::is_some){
                if config.privileges.drop_privileges {
                    PrivDrop::default()
                        .chroot(config.privileges.chroot_path.clone())
                        .user(config.privileges.user.clone())
                        .apply()
                        .context("Couldn't drop root privileges")?;
                }

                break
            }
        }
    }

    let response_channel_sender = ResponseChannelSender::new(out_message_senders);

    for i in 0..config.request_workers {
        let config = config.clone();
        let state = state.clone();
        let request_channel_receiver = request_channel_receiver.clone();
        let response_channel_sender = response_channel_sender.clone();
        let wakers = wakers.clone();

        Builder::new().name(format!("request-{:02}", i + 1)).spawn(move || {
            handler::run_request_worker(
                config,
                state,
                request_channel_receiver,
                response_channel_sender,
                wakers,
            );
        })?;
    }

    Ok(())
}

