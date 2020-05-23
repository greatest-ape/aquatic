use std::time::Duration;
use std::fs::File;
use std::io::Read;
use native_tls::{Identity, TlsAcceptor};

pub mod common;
pub mod config;
pub mod handler;
pub mod network;
pub mod protocol;
pub mod tasks;

use common::*;
use config::Config;


pub fn run(config: Config) -> anyhow::Result<()> {
    let opt_tls_acceptor = create_tls_acceptor(&config)?;

    let state = State::default();

    let (in_message_sender, in_message_receiver) = ::flume::unbounded();

    let mut out_message_senders = Vec::new();

    for i in 0..config.socket_workers {
        let config = config.clone();
        let in_message_sender = in_message_sender.clone();
        let opt_tls_acceptor = opt_tls_acceptor.clone();

        let (out_message_sender, out_message_receiver) = ::flume::unbounded();

        out_message_senders.push(out_message_sender);

        ::std::thread::spawn(move || {
            network::run_socket_worker(
                config,
                i,
                in_message_sender,
                out_message_receiver,
                opt_tls_acceptor
            );
        });
    }

    let out_message_sender = OutMessageSender::new(out_message_senders);

    {
        let config = config.clone();
        let state = state.clone();

        ::std::thread::spawn(move || {
            handler::run_request_worker(
                config,
                state,
                in_message_receiver,
                out_message_sender,
            );
        });
    }

    loop {
        ::std::thread::sleep(Duration::from_secs(config.cleaning.interval));

        tasks::clean_torrents(&state);
    }
}


pub fn create_tls_acceptor(
    config: &Config,
) -> anyhow::Result<Option<TlsAcceptor>> {
    if config.network.use_tls {
        let mut identity_bytes = Vec::new();
        let mut file = File::open(&config.network.tls_pkcs12_path)?;

        file.read_to_end(&mut identity_bytes)?;

        let identity = Identity::from_pkcs12(
            &mut identity_bytes,
            &config.network.tls_pkcs12_password
        )?;

        let acceptor = TlsAcceptor::new(identity)?;

        Ok(Some(acceptor))
    } else {
        Ok(None)
    }
}