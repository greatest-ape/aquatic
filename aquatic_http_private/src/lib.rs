mod common;
pub mod config;
mod workers;

use std::{collections::VecDeque, sync::Arc};

use aquatic_common::rustls_config::create_rustls_config;
use common::ChannelRequestSender;
use dotenv::dotenv;
use tokio::sync::mpsc::channel;

use config::Config;

pub const APP_NAME: &str = "aquatic_http_private: private HTTP/TLS BitTorrent tracker";
pub const APP_VERSION: &str = env!("CARGO_PKG_VERSION");

pub fn run(config: Config) -> anyhow::Result<()> {
    dotenv().ok();

    let tls_config = Arc::new(create_rustls_config(
        &config.network.tls_certificate_path,
        &config.network.tls_private_key_path,
    )?);

    let mut request_senders = Vec::new();
    let mut request_receivers = VecDeque::new();

    for _ in 0..config.request_workers {
        let (request_sender, request_receiver) = channel(config.worker_channel_size);

        request_senders.push(request_sender);
        request_receivers.push_back(request_receiver);
    }

    let mut handles = Vec::new();

    for _ in 0..config.socket_workers {
        let config = config.clone();
        let tls_config = tls_config.clone();
        let request_sender = ChannelRequestSender::new(request_senders.clone());

        let handle = ::std::thread::Builder::new()
            .name("socket".into())
            .spawn(move || {
                workers::socket::run_socket_worker(config, tls_config, request_sender)
            })?;

        handles.push(handle);
    }

    for _ in 0..config.request_workers {
        let config = config.clone();
        let request_receiver = request_receivers.pop_front().unwrap();

        let handle = ::std::thread::Builder::new()
            .name("request".into())
            .spawn(move || workers::request::run_request_worker(config, request_receiver))?;

        handles.push(handle);
    }

    for handle in handles {
        handle
            .join()
            .map_err(|err| anyhow::anyhow!("thread join error: {:?}", err))??;
    }

    Ok(())
}
