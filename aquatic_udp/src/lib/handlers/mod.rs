use std::net::SocketAddr;
use std::time::Duration;

use crossbeam_channel::{Receiver, Sender};
use parking_lot::MutexGuard;
use rand::{
    rngs::{SmallRng, StdRng},
    SeedableRng,
};

use aquatic_udp_protocol::*;

use crate::common::*;
use crate::config::Config;

mod announce;
mod scrape;

use announce::handle_announce_requests;
use scrape::handle_scrape_requests;

pub fn run_request_worker(
    state: State,
    config: Config,
    request_receiver: Receiver<(ConnectedRequest, SocketAddr)>,
    response_sender: Sender<(Response, SocketAddr)>,
) {
    let mut announce_requests: Vec<(AnnounceRequest, SocketAddr)> = Vec::new();
    let mut scrape_requests: Vec<(ScrapeRequest, SocketAddr)> = Vec::new();

    let mut responses: Vec<(Response, SocketAddr)> = Vec::new();

    let mut std_rng = StdRng::from_entropy();
    let mut small_rng = SmallRng::from_rng(&mut std_rng).unwrap();

    let timeout = Duration::from_micros(config.handlers.channel_recv_timeout_microseconds);

    loop {
        let mut opt_torrents = None;

        // Collect requests from channel, divide them by type
        //
        // Collect a maximum number of request. Stop collecting before that
        // number is reached if having waited for too long for a request, but
        // only if TorrentMaps mutex isn't locked.
        for i in 0..config.handlers.max_requests_per_iter {
            let (request, src): (ConnectedRequest, SocketAddr) = if i == 0 {
                match request_receiver.recv() {
                    Ok(r) => r,
                    Err(_) => break, // Really shouldn't happen
                }
            } else {
                match request_receiver.recv_timeout(timeout) {
                    Ok(r) => r,
                    Err(_) => {
                        if let Some(guard) = state.torrents.try_lock() {
                            opt_torrents = Some(guard);

                            break;
                        } else {
                            continue;
                        }
                    }
                }
            };

            match request {
                ConnectedRequest::Announce(r) => announce_requests.push((r, src)),
                ConnectedRequest::Scrape(r) => scrape_requests.push((r, src)),
            }
        }

        let mut torrents: MutexGuard<TorrentMaps> =
            opt_torrents.unwrap_or_else(|| state.torrents.lock());

        // Generate responses for announce and scrape requests

        handle_announce_requests(
            &config,
            &mut torrents,
            &mut small_rng,
            announce_requests.drain(..),
            &mut responses,
        );

        handle_scrape_requests(&mut torrents, scrape_requests.drain(..), &mut responses);

        for r in responses.drain(..) {
            if let Err(err) = response_sender.send(r) {
                ::log::error!("error sending response to channel: {}", err);
            }
        }
    }
}
