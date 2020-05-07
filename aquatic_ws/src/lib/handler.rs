use std::time::Duration;
use std::vec::Drain;

use hashbrown::HashMap;
use parking_lot::MutexGuard;

use crate::common::*;
use crate::protocol::*;


pub fn run_request_worker(
    state: State,
    in_message_receiver: InMessageReceiver,
    out_message_sender: OutMessageSender,
){
    let mut out_messages = Vec::new();

    let mut announce_requests = Vec::new();
    let mut scrape_requests = Vec::new();

    let timeout = Duration::from_micros(200);

    loop {
        let mut opt_torrent_map_guard: Option<MutexGuard<TorrentMap>> = None;

        for i in 0..1000 {
            if i == 0 {
                if let Ok((meta, in_message)) = in_message_receiver.recv(){
                    match in_message {
                        InMessage::AnnounceRequest(r) => {
                            announce_requests.push((meta, r));
                        },
                        InMessage::ScrapeRequest(r) => {
                            scrape_requests.push((meta, r));
                        },
                    }
                }
            } else {
                let res_in_message = in_message_receiver.recv_timeout(timeout);

                if let Ok((meta, in_message)) = res_in_message {
                    match in_message {
                        InMessage::AnnounceRequest(r) => {
                            announce_requests.push((meta, r));
                        },
                        InMessage::ScrapeRequest(r) => {
                            scrape_requests.push((meta, r));
                        },
                    }
                } else {
                    if let Some(torrent_guard) = state.torrents.try_lock(){
                        opt_torrent_map_guard = Some(torrent_guard);

                        break
                    }
                }
            };
        }

        let mut torrent_map_guard = opt_torrent_map_guard
            .unwrap_or_else(|| state.torrents.lock());

        handle_announce_requests(
            &mut torrent_map_guard,
            &mut out_messages,
            announce_requests.drain(..)
        );

        handle_scrape_requests(
            &mut torrent_map_guard,
            &mut out_messages,
            scrape_requests.drain(..)
        );

        ::std::mem::drop(torrent_map_guard);

        for (meta, out_message) in out_messages.drain(..){
            out_message_sender.send(meta, out_message);
        }
    }
}


pub fn handle_announce_requests(
    torrents: &mut TorrentMap,
    messages_out: &mut Vec<(ConnectionMeta, OutMessage)>,
    requests: Drain<(ConnectionMeta, AnnounceRequest)>,
){
    // if offers are set, fetch same number of peers, send offers to all of them
    // if answer is set, fetch that peer, send answer to it
    // finally, return announce response, I think
}


pub fn handle_scrape_requests(
    torrents: &mut TorrentMap,
    messages_out: &mut Vec<(ConnectionMeta, OutMessage)>,
    requests: Drain<(ConnectionMeta, ScrapeRequest)>,
){
    messages_out.extend(requests.map(|(meta, request)| {
        let num_info_hashes = request.info_hashes
            .as_ref()
            .map(|v| v.len())
            .unwrap_or(0);

        let mut response = ScrapeResponse {
            files: HashMap::with_capacity(num_info_hashes),
        };

        // If request.info_hashes is None, don't return scrape for all
        // torrents, even though that is done in reference server, it is
        // too expensive.
        if let Some(info_hashes) = request.info_hashes {
            for info_hash in info_hashes {
                if let Some(torrent_data) = torrents.get(&info_hash){
                    let stats = ScrapeStatistics {
                        complete: torrent_data.seeders,
                        downloaded: 0, // No implementation planned
                        incomplete: torrent_data.leechers,
                    };

                    response.files.insert(info_hash, stats);
                }
            }
        }

        (meta, OutMessage::ScrapeResponse(response))
    }));
}