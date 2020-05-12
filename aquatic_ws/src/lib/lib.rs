//! There is not much point in doing more work until more clarity on
//! exact protocol is achieved

pub mod common;
pub mod config;
pub mod handler;
pub mod network;
pub mod protocol;

use common::*;
use config::Config;


pub fn run(config: Config){
    let state = State::default();

    let (in_message_sender, in_message_receiver) = ::flume::unbounded();

    let mut out_message_senders = Vec::new();

    for i in 0..config.socket_workers {
        let config = config.clone();
        let in_message_sender = in_message_sender.clone();

        let (out_message_sender, out_message_receiver) = ::flume::unbounded();

        out_message_senders.push(out_message_sender);

        ::std::thread::spawn(move || {
            network::run_socket_worker(
                config,
                i,
                in_message_sender,
                out_message_receiver,
            );
        });
    }

    let out_message_sender = OutMessageSender::new(out_message_senders);

    ::std::thread::spawn(move || {
        handler::run_request_worker(
            config,
            state,
            in_message_receiver,
            out_message_sender,
        );
    });

    loop {
        ::std::thread::sleep(::std::time::Duration::from_secs(60));
    }
}