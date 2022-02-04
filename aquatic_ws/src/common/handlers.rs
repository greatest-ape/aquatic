use aquatic_common::extract_response_peers;
use hashbrown::HashMap;
use rand::rngs::SmallRng;

use aquatic_ws_protocol::*;

use crate::common::*;
use crate::config::Config;

pub fn handle_announce_request(
    config: &Config,
    rng: &mut SmallRng,
    torrent_maps: &mut TorrentMaps,
    out_messages: &mut Vec<(ConnectionMeta, OutMessage)>,
    valid_until: ValidUntil,
    request_sender_meta: ConnectionMeta,
    request: AnnounceRequest,
) {
    let torrent_data: &mut TorrentData = if request_sender_meta.peer_addr.is_ipv4() {
        torrent_maps.ipv4.entry(request.info_hash).or_default()
    } else {
        torrent_maps.ipv6.entry(request.info_hash).or_default()
    };

    // If there is already a peer with this peer_id, check that socket
    // addr is same as that of request sender. Otherwise, ignore request.
    // Since peers have access to each others peer_id's, they could send
    // requests using them, causing all sorts of issues.
    if let Some(previous_peer) = torrent_data.peers.get(&request.peer_id) {
        if request_sender_meta.peer_addr != previous_peer.connection_meta.peer_addr {
            return;
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

            out_messages.push((
                offer_receiver.connection_meta,
                OutMessage::Offer(middleman_offer),
            ));
            ::log::trace!(
                "sending middleman offer to {:?}",
                offer_receiver.connection_meta
            );
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

            out_messages.push((
                answer_receiver.connection_meta,
                OutMessage::Answer(middleman_answer),
            ));
            ::log::trace!(
                "sending middleman answer to {:?}",
                answer_receiver.connection_meta
            );
        }
    }

    let out_message = OutMessage::AnnounceResponse(AnnounceResponse {
        action: AnnounceAction,
        info_hash: request.info_hash,
        complete: torrent_data.num_seeders,
        incomplete: torrent_data.num_leechers,
        announce_interval: config.protocol.peer_announce_interval,
    });

    out_messages.push((request_sender_meta, out_message));
}

pub fn handle_scrape_request(
    config: &Config,
    torrent_maps: &mut TorrentMaps,
    out_messages: &mut Vec<(ConnectionMeta, OutMessage)>,
    meta: ConnectionMeta,
    request: ScrapeRequest,
) {
    let info_hashes = if let Some(info_hashes) = request.info_hashes {
        info_hashes.as_vec()
    } else {
        return;
    };

    let num_to_take = info_hashes.len().min(config.protocol.max_scrape_torrents);

    let mut out_message = ScrapeResponse {
        action: ScrapeAction,
        files: HashMap::with_capacity(num_to_take),
    };

    let torrent_map: &mut TorrentMap = if meta.peer_addr.is_ipv4() {
        &mut torrent_maps.ipv4
    } else {
        &mut torrent_maps.ipv6
    };

    for info_hash in info_hashes.into_iter().take(num_to_take) {
        if let Some(torrent_data) = torrent_map.get(&info_hash) {
            let stats = ScrapeStatistics {
                complete: torrent_data.num_seeders,
                downloaded: 0, // No implementation planned
                incomplete: torrent_data.num_leechers,
            };

            out_message.files.insert(info_hash, stats);
        }
    }

    out_messages.push((meta, OutMessage::ScrapeResponse(out_message)));
}
