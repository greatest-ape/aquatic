use std::net::SocketAddr;
use std::time::Duration;

use aquatic_common::ValidUntil;
use crossbeam_channel::{Receiver, Sender};
use rand::{rngs::SmallRng, SeedableRng};

use aquatic_udp_protocol::*;

use crate::common::handlers::*;
use crate::config::Config;
use crate::mio::common::*;

pub fn run_request_worker(
    state: State,
    config: Config,
    request_receiver: Receiver<(ConnectedRequest, SocketAddr)>,
    response_sender: Sender<(ConnectedResponse, SocketAddr)>,
) {
    let mut announce_requests: Vec<(AnnounceRequest, SocketAddr)> = Vec::new();
    let mut scrape_requests: Vec<(ScrapeRequest, SocketAddr)> = Vec::new();
    let mut responses: Vec<(ConnectedResponse, SocketAddr)> = Vec::new();

    let mut small_rng = SmallRng::from_entropy();

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
                ConnectedRequest::Announce(request) => announce_requests.push((request, src)),
                ConnectedRequest::Scrape { request, .. } => scrape_requests.push((request, src)),
            }
        }

        // Generate responses for announce and scrape requests, then drop MutexGuard.
        {
            let mut torrents = opt_torrents.unwrap_or_else(|| state.torrents.lock());

            let peer_valid_until = ValidUntil::new(config.cleaning.max_peer_age);

            responses.extend(announce_requests.drain(..).map(|(request, src)| {
                let response = handle_announce_request(
                    &config,
                    &mut small_rng,
                    &mut torrents,
                    request,
                    src,
                    peer_valid_until,
                );

                (ConnectedResponse::Announce(response), src)
            }));

            responses.extend(scrape_requests.drain(..).map(|(request, src)| {
                let response = ConnectedResponse::Scrape {
                    response: handle_scrape_request(&mut torrents, src, request),
                    original_indices: Vec::new(),
                };

                (response, src)
            }));
        }

        for r in responses.drain(..) {
            if let Err(err) = response_sender.send(r) {
                ::log::error!("error sending response to channel: {}", err);
            }
        }
    }
}
