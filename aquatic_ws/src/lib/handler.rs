use std::time::Duration;

use hashbrown::HashMap;

use crate::common::*;
use crate::protocol::*;


pub fn run_request_worker(
    state: State,
    in_message_receiver: InMessageReceiver,
    out_message_sender: OutMessageSender,
){
    let mut in_messages = Vec::new();
    let mut out_messages = Vec::new();

    let timeout = Duration::from_micros(200);

    for i in 0..1000 {
        if i == 0 {
            if let Ok((meta, in_message)) = in_message_receiver.recv(){
                in_messages.push((meta, in_message));
            }
        } else {
            let res_in_message = in_message_receiver.recv_timeout(timeout);

            if let Ok((meta, in_message)) = res_in_message {
                in_messages.push((meta, in_message));
            } else {
                break
            }
        };
    }

    // TODO: lock torrent mutex

    // This should be separate function. Possibly requests should be separated
    // above and one announce and one scrape handler used
    for (meta, in_message) in in_messages.drain(..){
        // announce requests:
        // if offers are set, fetch same number of peers, send offers to all of them
        // if answer is set, fetch that peer, send answer to it
        // finally, return announce response, seemingly

        // scrape: just fetch stats for all info_hashes sent

        let out_message = OutMessage::ScrapeResponse(ScrapeResponse {
            files: HashMap::new(),
        });

        out_messages.push((meta, out_message));
    }

    // TODO: unlock torrent mutex

    for (meta, out_message) in out_messages.drain(..){
        out_message_sender.send(meta, out_message);
    }
}