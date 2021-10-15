use std::sync::Arc;
use std::time::Duration;
use std::vec::Drain;

use hashbrown::HashMap;
use mio::Waker;
use parking_lot::MutexGuard;
use rand::{rngs::SmallRng, Rng, SeedableRng};

use aquatic_common::extract_response_peers;
use aquatic_ws_protocol::*;

use crate::common::*;
use crate::config::Config;

pub fn run_request_worker(
    config: Config,
    state: State,
    in_message_receiver: InMessageReceiver,
    out_message_sender: OutMessageSender,
    wakers: Vec<Arc<Waker>>,
) {
    let mut wake_socket_workers: Vec<bool> = (0..config.socket_workers).map(|_| false).collect();

    let mut announce_requests = Vec::new();
    let mut scrape_requests = Vec::new();

    let mut rng = SmallRng::from_entropy();

    let timeout = Duration::from_micros(config.handlers.channel_recv_timeout_microseconds);

    loop {
        let mut opt_torrent_map_guard: Option<MutexGuard<TorrentMaps>> = None;

        for i in 0..config.handlers.max_requests_per_iter {
            let opt_in_message = if i == 0 {
                in_message_receiver.recv().ok()
            } else {
                in_message_receiver.recv_timeout(timeout).ok()
            };

            match opt_in_message {
                Some((meta, InMessage::AnnounceRequest(r))) => {
                    announce_requests.push((meta, r));
                }
                Some((meta, InMessage::ScrapeRequest(r))) => {
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
            &out_message_sender,
            &mut wake_socket_workers,
            announce_requests.drain(..),
        );

        handle_scrape_requests(
            &config,
            &mut torrent_map_guard,
            &out_message_sender,
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
    out_message_sender: &OutMessageSender,
    wake_socket_workers: &mut Vec<bool>,
    requests: Drain<(ConnectionMeta, AnnounceRequest)>,
) {
    let valid_until = ValidUntil::new(config.cleaning.max_peer_age);

    for (request_sender_meta, request) in requests {
        let info_hash_allowed = torrent_maps
            .access_list
            .allows(config.access_list.mode, &request.info_hash.0);
        
        if !info_hash_allowed {
            let response = OutMessage::ErrorResponse(ErrorResponse {
                failure_reason: "Info hash not allowed".into(),
                action: Some(ErrorResponseAction::Announce),
                info_hash: Some(request.info_hash),
            });
    
            out_message_sender.send(request_sender_meta, response);
            wake_socket_workers[request_sender_meta.worker_index] = true;

            continue;
        }

        let torrent_data: &mut TorrentData = if request_sender_meta.converted_peer_ip.is_ipv4() {
            torrent_maps.ipv4.entry(request.info_hash).or_default()
        } else {
            torrent_maps.ipv6.entry(request.info_hash).or_default()
        };

        // If there is already a peer with this peer_id, check that socket
        // addr is same as that of request sender. Otherwise, ignore request.
        // Since peers have access to each others peer_id's, they could send
        // requests using them, causing all sorts of issues. Checking naive
        // (non-converted) socket addresses is enough, since state is split
        // on converted peer ip.
        if let Some(previous_peer) = torrent_data.peers.get(&request.peer_id) {
            if request_sender_meta.naive_peer_addr != previous_peer.connection_meta.naive_peer_addr
            {
                continue;
            }
        }

        ::log::trace!("received request from {:?}", request_sender_meta);

        // Insert/update/remove peer who sent this request
        {
            let peer_status = PeerStatus::from_event_and_bytes_left(
                request.event.unwrap_or_default(),
                request.bytes_left,
            );

            let peer = Peer {
                connection_meta: request_sender_meta,
                status: peer_status,
                valid_until,
            };

            let opt_removed_peer = match peer_status {
                PeerStatus::Leeching => {
                    torrent_data.num_leechers += 1;

                    torrent_data.peers.insert(request.peer_id, peer)
                }
                PeerStatus::Seeding => {
                    torrent_data.num_seeders += 1;

                    torrent_data.peers.insert(request.peer_id, peer)
                }
                PeerStatus::Stopped => torrent_data.peers.remove(&request.peer_id),
            };

            match opt_removed_peer.map(|peer| peer.status) {
                Some(PeerStatus::Leeching) => {
                    torrent_data.num_leechers -= 1;
                }
                Some(PeerStatus::Seeding) => {
                    torrent_data.num_seeders -= 1;
                }
                _ => {}
            }
        }

        // If peer sent offers, send them on to random peers
        if let Some(offers) = request.offers {
            // FIXME: config: also maybe check this when parsing request
            let max_num_peers_to_take = offers.len().min(config.protocol.max_offers);

            #[inline]
            fn f(peer: &Peer) -> Peer {
                *peer
            }

            let offer_receivers: Vec<Peer> = extract_response_peers(
                rng,
                &torrent_data.peers,
                max_num_peers_to_take,
                request.peer_id,
                f,
            );

            for (offer, offer_receiver) in offers.into_iter().zip(offer_receivers) {
                let middleman_offer = MiddlemanOfferToPeer {
                    action: AnnounceAction,
                    info_hash: request.info_hash,
                    peer_id: request.peer_id,
                    offer: offer.offer,
                    offer_id: offer.offer_id,
                };

                out_message_sender.send(
                    offer_receiver.connection_meta,
                    OutMessage::Offer(middleman_offer),
                );
                ::log::trace!(
                    "sent middleman offer to {:?}",
                    offer_receiver.connection_meta
                );
                wake_socket_workers[offer_receiver.connection_meta.worker_index] = true;
            }
        }

        // If peer sent answer, send it on to relevant peer
        if let (Some(answer), Some(answer_receiver_id), Some(offer_id)) =
            (request.answer, request.to_peer_id, request.offer_id)
        {
            if let Some(answer_receiver) = torrent_data.peers.get(&answer_receiver_id) {
                let middleman_answer = MiddlemanAnswerToPeer {
                    action: AnnounceAction,
                    peer_id: request.peer_id,
                    info_hash: request.info_hash,
                    answer,
                    offer_id,
                };

                out_message_sender.send(
                    answer_receiver.connection_meta,
                    OutMessage::Answer(middleman_answer),
                );
                ::log::trace!(
                    "sent middleman answer to {:?}",
                    answer_receiver.connection_meta
                );
                wake_socket_workers[answer_receiver.connection_meta.worker_index] = true;
            }
        }

        let response = OutMessage::AnnounceResponse(AnnounceResponse {
            action: AnnounceAction,
            info_hash: request.info_hash,
            complete: torrent_data.num_seeders,
            incomplete: torrent_data.num_leechers,
            announce_interval: config.protocol.peer_announce_interval,
        });

        out_message_sender.send(request_sender_meta, response);
        wake_socket_workers[request_sender_meta.worker_index] = true;
    }
}

pub fn handle_scrape_requests(
    config: &Config,
    torrent_maps: &mut TorrentMaps,
    out_message_sender: &OutMessageSender,
    wake_socket_workers: &mut Vec<bool>,
    requests: Drain<(ConnectionMeta, ScrapeRequest)>,
) {
    for (meta, request) in requests {
        let info_hashes = if let Some(info_hashes) = request.info_hashes {
            info_hashes.as_vec()
        } else {
            continue;
        };

        let num_to_take = info_hashes.len().min(config.protocol.max_scrape_torrents);

        let mut response = ScrapeResponse {
            action: ScrapeAction,
            files: HashMap::with_capacity(num_to_take),
        };

        let torrent_map: &mut TorrentMap = if meta.converted_peer_ip.is_ipv4() {
            &mut torrent_maps.ipv4
        } else {
            &mut torrent_maps.ipv6
        };

        // If request.info_hashes is empty, don't return scrape for all
        // torrents, even though reference server does it. It is too expensive.
        for info_hash in info_hashes.into_iter().take(num_to_take) {
            if let Some(torrent_data) = torrent_map.get(&info_hash) {
                let stats = ScrapeStatistics {
                    complete: torrent_data.num_seeders,
                    downloaded: 0, // No implementation planned
                    incomplete: torrent_data.num_leechers,
                };

                response.files.insert(info_hash, stats);
            }
        }

        out_message_sender.send(meta, OutMessage::ScrapeResponse(response));
        wake_socket_workers[meta.worker_index] = true;
    }
}
