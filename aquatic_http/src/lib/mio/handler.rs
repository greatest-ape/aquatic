use std::sync::Arc;
use std::time::Duration;

use mio::Waker;
use parking_lot::MutexGuard;
use rand::{rngs::SmallRng, SeedableRng};

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

        let valid_until = ValidUntil::new(config.cleaning.max_peer_age);

        for (meta, request) in announce_requests.drain(..) {
            let response = handle_announce_request(
                &config,
                &mut rng,
                &mut torrent_map_guard,
                valid_until,
                meta,
                request
            );

            response_channel_sender.send(meta, Response::Announce(response));
            wake_socket_workers[meta.worker_index] = true;
        }

        for (meta, request) in scrape_requests.drain(..) {
            let response = handle_scrape_request(&config, &mut torrent_map_guard, (meta, request));

            response_channel_sender.send(meta, Response::Scrape(response));
            wake_socket_workers[meta.worker_index] = true;
        }

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
