use std::sync::Arc;
use std::time::Duration;
use std::vec::Drain;

use mio::Waker;
use parking_lot::MutexGuard;
use rand::{rngs::SmallRng, Rng, SeedableRng};

use aquatic_http_protocol::request::*;
use aquatic_http_protocol::response::*;

use crate::common::handlers::{handle_announce_request, handle_scrape_request};
use crate::common::*;
use crate::config::Config;
use super::common::*;

pub fn run_request_worker(
    config: Config,
    state: State,
    request_channel_receiver: RequestChannelReceiver,
    response_channel_sender: ResponseChannelSender,
    wakers: Vec<Arc<Waker>>,
) {
    let mut wake_socket_workers: Vec<bool> = (0..config.socket_workers).map(|_| false).collect();

    let mut announce_requests = Vec::new();
    let mut scrape_requests = Vec::new();

    let mut rng = SmallRng::from_entropy();

    let timeout = Duration::from_micros(config.handlers.channel_recv_timeout_microseconds);

    loop {
        let mut opt_torrent_map_guard: Option<MutexGuard<TorrentMaps>> = None;

        // If torrent state mutex is locked, just keep collecting requests
        // and process them later. This can happen with either multiple
        // request workers or while cleaning is underway.
        for i in 0..config.handlers.max_requests_per_iter {
            let opt_in_message = if i == 0 {
                request_channel_receiver.recv().ok()
            } else {
                request_channel_receiver.recv_timeout(timeout).ok()
            };

            match opt_in_message {
                Some((meta, Request::Announce(r))) => {
                    announce_requests.push((meta, r));
                }
                Some((meta, Request::Scrape(r))) => {
                    scrape_requests.push((meta, r));
                }
                None => {
                    if let Some(torrent_guard) = state.torrent_maps.try_lock() {
                        opt_torrent_map_guard = Some(torrent_guard);

                        break;
                    }
                }
            }
        }

        let mut torrent_map_guard =
            opt_torrent_map_guard.unwrap_or_else(|| state.torrent_maps.lock());

        handle_announce_requests(
            &config,
            &mut rng,
            &mut torrent_map_guard,
            &response_channel_sender,
            &mut wake_socket_workers,
            announce_requests.drain(..),
        );

        handle_scrape_requests(
            &config,
            &mut torrent_map_guard,
            &response_channel_sender,
            &mut wake_socket_workers,
            scrape_requests.drain(..),
        );

        for (worker_index, wake) in wake_socket_workers.iter_mut().enumerate() {
            if *wake {
                if let Err(err) = wakers[worker_index].wake() {
                    ::log::error!("request handler couldn't wake poll: {:?}", err);
                }

                *wake = false;
            }
        }
    }
}

pub fn handle_announce_requests(
    config: &Config,
    rng: &mut impl Rng,
    torrent_maps: &mut TorrentMaps,
    response_channel_sender: &ResponseChannelSender,
    wake_socket_workers: &mut Vec<bool>,
    requests: Drain<(ConnectionMeta, AnnounceRequest)>,
) {
    let valid_until = ValidUntil::new(config.cleaning.max_peer_age);

    for (meta, request) in requests {
        let response = handle_announce_request(config, rng, torrent_maps, valid_until, meta, request);

        response_channel_sender.send(meta, Response::Announce(response));
        wake_socket_workers[meta.worker_index] = true;
    }
}

pub fn handle_scrape_requests(
    config: &Config,
    torrent_maps: &mut TorrentMaps,
    response_channel_sender: &ResponseChannelSender,
    wake_socket_workers: &mut Vec<bool>,
    requests: Drain<(ConnectionMeta, ScrapeRequest)>,
) {
    for (meta, request) in requests {
        let response = handle_scrape_request(config, torrent_maps, (meta, request));

        response_channel_sender.send(meta, Response::Scrape(response));
        wake_socket_workers[meta.worker_index] = true;
    }
}
