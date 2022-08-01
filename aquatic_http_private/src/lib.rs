mod common;
pub mod config;
mod workers;

use std::{collections::VecDeque, sync::Arc};

use aquatic_common::{
    privileges::PrivilegeDropper, rustls_config::create_rustls_config, PanicSentinelWatcher,
    ServerStartInstant,
};
use common::ChannelRequestSender;
use dotenv::dotenv;
use signal_hook::{consts::SIGTERM, iterator::Signals};
use tokio::sync::mpsc::channel;

use config::Config;

pub const APP_NAME: &str = "aquatic_http_private: private HTTP/TLS BitTorrent tracker";
pub const APP_VERSION: &str = env!("CARGO_PKG_VERSION");

pub fn run(config: Config) -> anyhow::Result<()> {
    let mut signals = Signals::new([SIGTERM])?;

    dotenv().ok();

    let tls_config = Arc::new(create_rustls_config(
        &config.network.tls_certificate_path,
        &config.network.tls_private_key_path,
    )?);

    let mut request_senders = Vec::new();
    let mut request_receivers = VecDeque::new();

    for _ in 0..config.swarm_workers {
        let (request_sender, request_receiver) = channel(config.worker_channel_size);

        request_senders.push(request_sender);
        request_receivers.push_back(request_receiver);
    }

    let (sentinel_watcher, sentinel) = PanicSentinelWatcher::create_with_sentinel();
    let priv_dropper = PrivilegeDropper::new(config.privileges.clone(), config.socket_workers);

    let server_start_instant = ServerStartInstant::new();

    let mut handles = Vec::new();

    for _ in 0..config.socket_workers {
        let sentinel = sentinel.clone();
        let config = config.clone();
        let tls_config = tls_config.clone();
        let request_sender = ChannelRequestSender::new(request_senders.clone());
        let priv_dropper = priv_dropper.clone();

        let handle = ::std::thread::Builder::new()
            .name("socket".into())
            .spawn(move || {
                workers::socket::run_socket_worker(
                    sentinel,
                    config,
                    tls_config,
                    request_sender,
                    priv_dropper,
                )
            })?;

        handles.push(handle);
    }

    for _ in 0..config.swarm_workers {
        let sentinel = sentinel.clone();
        let config = config.clone();
        let request_receiver = request_receivers.pop_front().unwrap();

        let handle = ::std::thread::Builder::new()
            .name("request".into())
            .spawn(move || {
                workers::swarm::run_swarm_worker(
                    sentinel,
                    config,
                    request_receiver,
                    server_start_instant,
                )
            })?;

        handles.push(handle);
    }

    for signal in &mut signals {
        match signal {
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
