mod common;
pub mod config;
mod workers;

use std::collections::VecDeque;

use common::ChannelRequestSender;
use dotenv::dotenv;
use tokio::sync::mpsc::channel;

pub const APP_NAME: &str = "aquatic_http_private: private HTTP/TLS BitTorrent tracker";
pub const APP_VERSION: &str = env!("CARGO_PKG_VERSION");

pub fn run(config: config::Config) -> anyhow::Result<()> {
    dotenv().ok();

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
        let request_sender = ChannelRequestSender::new(request_senders.clone());

        let handle = ::std::thread::Builder::new()
            .name("socket".into())
            .spawn(move || workers::socket::run_socket_worker(config, request_sender))?;

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
