//! There is not much point in doing more work until more clarity on
//! exact protocol is achieved

pub mod common;
pub mod handler;
pub mod network;
pub mod protocol;

use common::*;


pub fn run(){
    let state = State::default();

    let (in_message_sender, in_message_receiver): (InMessageSender, InMessageReceiver) = ::flume::unbounded();

    let mut out_message_senders = Vec::new();

    for i in 0..2 {
        let in_message_sender = in_message_sender.clone();

        let (out_message_sender, out_message_receiver) = ::flume::unbounded();

        out_message_senders.push(out_message_sender);

        ::std::thread::spawn(move || {
            network::run_socket_worker(
                i,
                in_message_sender,
                out_message_receiver,
            );
        });
    }

    let out_message_sender = OutMessageSender::new(out_message_senders);

    ::std::thread::spawn(move || {
        handler::run_request_worker(
            state,
            in_message_receiver,
            out_message_sender,
        );
    });

    loop {

    }
}