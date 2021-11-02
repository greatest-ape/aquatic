use std::cell::RefCell;
use std::rc::Rc;
use std::time::Duration;

use aquatic_common::access_list::AccessList;
use aquatic_common::extract_response_peers;
use futures_lite::StreamExt;
use glommio::channels::channel_mesh::{MeshBuilder, Partial, Role, Senders};
use glommio::enclose;
use glommio::prelude::*;
use glommio::timer::TimerActionRepeat;
use hashbrown::HashMap;
use rand::{rngs::SmallRng, Rng, SeedableRng};

use aquatic_ws_protocol::*;

use crate::common::*;
use crate::config::Config;

pub async fn run_request_worker(
    config: Config,
    in_message_mesh_builder: MeshBuilder<(ConnectionMeta, InMessage), Partial>,
    out_message_mesh_builder: MeshBuilder<(ConnectionMeta, OutMessage), Partial>,
    access_list: AccessList,
) {
    let (_, mut in_message_receivers) = in_message_mesh_builder.join(Role::Consumer).await.unwrap();
    let (out_message_senders, _) = out_message_mesh_builder.join(Role::Producer).await.unwrap();

    let out_message_senders = Rc::new(out_message_senders);

    let torrents = Rc::new(RefCell::new(TorrentMaps::default()));
    let access_list = Rc::new(RefCell::new(access_list));

    // Periodically clean torrents and update access list
    TimerActionRepeat::repeat(enclose!((config, torrents, access_list) move || {
        enclose!((config, torrents, access_list) move || async move {
            update_access_list(&config, access_list.clone()).await;

            torrents.borrow_mut().clean(&config, &*access_list.borrow());

            Some(Duration::from_secs(config.cleaning.interval))
        })()
    }));

    let mut handles = Vec::new();

    for (_, receiver) in in_message_receivers.streams() {
        let handle = spawn_local(handle_request_stream(
            config.clone(),
            torrents.clone(),
            out_message_senders.clone(),
            receiver,
        ))
        .detach();

        handles.push(handle);
    }

    for handle in handles {
        handle.await;
    }
}

async fn handle_request_stream<S>(
    config: Config,
    torrents: Rc<RefCell<TorrentMaps>>,
    out_message_senders: Rc<Senders<(ConnectionMeta, OutMessage)>>,
    mut stream: S,
) where
    S: futures_lite::Stream<Item = (ConnectionMeta, InMessage)> + ::std::marker::Unpin,
{
    let mut rng = SmallRng::from_entropy();

    let max_peer_age = config.cleaning.max_peer_age;
    let peer_valid_until = Rc::new(RefCell::new(ValidUntil::new(max_peer_age)));

    TimerActionRepeat::repeat(enclose!((peer_valid_until) move || {
        enclose!((peer_valid_until) move || async move {
            *peer_valid_until.borrow_mut() = ValidUntil::new(max_peer_age);

            Some(Duration::from_secs(1))
        })()
    }));

    let mut out_messages = Vec::new();

    while let Some((meta, in_message)) = stream.next().await {
        match in_message {
            InMessage::AnnounceRequest(request) => handle_announce_request(
                &config,
                &mut rng,
                &mut torrents.borrow_mut(),
                &mut out_messages,
                peer_valid_until.borrow().to_owned(),
                meta,
                request,
            ),
            InMessage::ScrapeRequest(request) => handle_scrape_request(
                &config,
                &mut torrents.borrow_mut(),
                &mut out_messages,
                meta,
                request,
            ),
        };

        for (meta, out_message) in out_messages.drain(..) {
            out_message_senders
                .send_to(meta.out_message_consumer_id.0, (meta, out_message))
                .await
                .expect("failed sending out_message to socket worker");
        }

        yield_if_needed().await;
    }
}

pub fn handle_announce_request(
    config: &Config,
    rng: &mut impl Rng,
    torrent_maps: &mut TorrentMaps,
    out_messages: &mut Vec<(ConnectionMeta, OutMessage)>,
    valid_until: ValidUntil,
    request_sender_meta: ConnectionMeta,
    request: AnnounceRequest,
) {
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
        if request_sender_meta.naive_peer_addr != previous_peer.connection_meta.naive_peer_addr {
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

    let torrent_map: &mut TorrentMap = if meta.converted_peer_ip.is_ipv4() {
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
