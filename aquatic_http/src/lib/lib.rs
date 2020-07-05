use std::time::Duration;
use std::sync::Arc;
use std::thread::Builder;

use anyhow::Context;
use parking_lot::Mutex;
use privdrop::PrivDrop;

use aquatic_common_tcp::network::utils::create_tls_acceptor;

pub mod common;
pub mod config;
pub mod handler;
pub mod network;
pub mod protocol;
pub mod tasks;

use common::*;
use config::Config;


pub fn run(config: Config) -> anyhow::Result<()> {
    let opt_tls_acceptor = create_tls_acceptor(&config.network.tls)?;

    let state = State::default();

    let (request_channel_sender, request_channel_receiver) = ::flume::unbounded();

    let mut out_message_senders = Vec::new();

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

        let (response_channel_sender, response_channel_receiver) = ::flume::unbounded();

        out_message_senders.push(response_channel_sender);

        Builder::new().name(format!("socket-{:02}", i + 1)).spawn(move || {
            network::run_socket_worker(
                config,
                i,
                socket_worker_statuses,
                request_channel_sender,
                response_channel_receiver,
                opt_tls_acceptor
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

    {
        let config = config.clone();
        let state = state.clone();

        Builder::new().name("request".to_string()).spawn(move || {
            handler::run_request_worker(
                config,
                state,
                request_channel_receiver,
                response_channel_sender,
            );
        })?;
    }

    loop {
        ::std::thread::sleep(Duration::from_secs(config.cleaning.interval));

        tasks::clean_torrents(&state);
    }
}

