use std::cell::RefCell;
use std::rc::Rc;
use std::sync::Arc;
use std::time::Duration;

use aquatic_common::access_list::{create_access_list_cache, AccessListArcSwap, AccessListCache};
use futures::StreamExt;
use glommio::channels::channel_mesh::{MeshBuilder, Partial, Role, Senders};
use glommio::enclose;
use glommio::prelude::*;
use glommio::timer::TimerActionRepeat;
use hashbrown::HashMap;
use rand::{rngs::SmallRng, SeedableRng};

use aquatic_common::{
    extract_response_peers, IndexMap, PanicSentinel, SecondsSinceServerStart, ServerStartInstant,
};
use aquatic_ws_protocol::*;

use crate::common::*;
use crate::config::Config;
use crate::SHARED_IN_CHANNEL_SIZE;

#[cfg(feature = "metrics")]
thread_local! { static WORKER_INDEX: ::std::cell::Cell<usize> = Default::default() }

#[derive(PartialEq, Eq, Clone, Copy, Debug)]
enum PeerStatus {
    Seeding,
    Leeching,
    Stopped,
}

impl PeerStatus {
    /// Determine peer status from announce event and number of bytes left.
    ///
    /// Likely, the last branch will be taken most of the time.
    #[inline]
    fn from_event_and_bytes_left(event: AnnounceEvent, opt_bytes_left: Option<usize>) -> Self {
        if let AnnounceEvent::Stopped = event {
            Self::Stopped
        } else if let Some(0) = opt_bytes_left {
            Self::Seeding
        } else {
            Self::Leeching
        }
    }
}

#[derive(Clone, Copy)]
struct Peer {
    pub consumer_id: ConsumerId,
    pub connection_id: ConnectionId,
    pub seeder: bool,
    pub valid_until: ValidUntil,
}

type PeerMap = IndexMap<PeerId, Peer>;

struct TorrentData {
    pub peers: PeerMap,
    pub num_seeders: usize,
}

impl Default for TorrentData {
    #[inline]
    fn default() -> Self {
        Self {
            peers: Default::default(),
            num_seeders: 0,
        }
    }
}

impl TorrentData {
    pub fn remove_peer(&mut self, peer_id: PeerId) {
        if let Some(peer) = self.peers.remove(&peer_id) {
            if peer.seeder {
                self.num_seeders -= 1;
            }
        }
    }

    pub fn num_leechers(&self) -> usize {
        self.peers.len() - self.num_seeders
    }
}

type TorrentMap = IndexMap<InfoHash, TorrentData>;

#[derive(Default)]
struct TorrentMaps {
    pub ipv4: TorrentMap,
    pub ipv6: TorrentMap,
}

impl TorrentMaps {
    fn clean(
        &mut self,
        config: &Config,
        access_list: &Arc<AccessListArcSwap>,
        server_start_instant: ServerStartInstant,
    ) {
        let mut access_list_cache = create_access_list_cache(access_list);
        let now = server_start_instant.seconds_elapsed();

        Self::clean_torrent_map(config, &mut access_list_cache, &mut self.ipv4, now, "4");
        Self::clean_torrent_map(config, &mut access_list_cache, &mut self.ipv6, now, "6");
    }

    fn clean_torrent_map(
        config: &Config,
        access_list_cache: &mut AccessListCache,
        torrent_map: &mut TorrentMap,
        now: SecondsSinceServerStart,
        ip_version: &'static str,
    ) {
        let mut total_num_peers = 0u64;

        torrent_map.retain(|info_hash, torrent_data| {
            if !access_list_cache
                .load()
                .allows(config.access_list.mode, &info_hash.0)
            {
                return false;
            }

            let num_seeders = &mut torrent_data.num_seeders;

            torrent_data.peers.retain(|_, peer| {
                let keep = peer.valid_until.valid(now);

                if (!keep) & peer.seeder {
                    *num_seeders -= 1;
                }

                keep
            });

            total_num_peers += torrent_data.peers.len() as u64;

            !torrent_data.peers.is_empty()
        });

        torrent_map.shrink_to_fit();

        let total_num_peers = total_num_peers as f64;

        #[cfg(feature = "metrics")]
        ::metrics::gauge!(
            "aquatic_peers",
            total_num_peers,
            "ip_version" => ip_version,
            "worker_index" => WORKER_INDEX.with(|index| index.get()).to_string(),
        );
    }
}

pub async fn run_swarm_worker(
    _sentinel: PanicSentinel,
    config: Config,
    state: State,
    control_message_mesh_builder: MeshBuilder<SwarmControlMessage, Partial>,
    in_message_mesh_builder: MeshBuilder<(InMessageMeta, InMessage), Partial>,
    out_message_mesh_builder: MeshBuilder<(OutMessageMeta, OutMessage), Partial>,
    server_start_instant: ServerStartInstant,
    worker_index: usize,
) {
    #[cfg(feature = "metrics")]
    WORKER_INDEX.with(|index| index.set(worker_index));

    let (_, mut control_message_receivers) = control_message_mesh_builder
        .join(Role::Consumer)
        .await
        .unwrap();

    let (_, mut in_message_receivers) = in_message_mesh_builder.join(Role::Consumer).await.unwrap();
    let (out_message_senders, _) = out_message_mesh_builder.join(Role::Producer).await.unwrap();

    let out_message_senders = Rc::new(out_message_senders);

    let torrents = Rc::new(RefCell::new(TorrentMaps::default()));
    let access_list = state.access_list;

    // Periodically clean torrents
    TimerActionRepeat::repeat(enclose!((config, torrents, access_list) move || {
        enclose!((config, torrents, access_list) move || async move {
            torrents.borrow_mut().clean(&config, &access_list, server_start_instant);

            Some(Duration::from_secs(config.cleaning.torrent_cleaning_interval))
        })()
    }));

    // Periodically update torrent count metrics
    #[cfg(feature = "metrics")]
    TimerActionRepeat::repeat(enclose!((config, torrents) move || {
        enclose!((config, torrents) move || async move {
            let torrents = torrents.borrow_mut();

            ::metrics::gauge!(
                "aquatic_torrents",
                torrents.ipv4.len() as f64,
                "ip_version" => "4",
                "worker_index" => worker_index.to_string(),
            );
            ::metrics::gauge!(
                "aquatic_torrents",
                torrents.ipv6.len() as f64,
                "ip_version" => "6",
                "worker_index" => worker_index.to_string(),
            );

            Some(Duration::from_secs(config.metrics.torrent_count_update_interval))
        })()
    }));

    let mut handles = Vec::new();

    for (_, receiver) in control_message_receivers.streams() {
        let handle =
            spawn_local(handle_control_message_stream(torrents.clone(), receiver)).detach();

        handles.push(handle);
    }

    for (_, receiver) in in_message_receivers.streams() {
        let handle = spawn_local(handle_request_stream(
            config.clone(),
            torrents.clone(),
            server_start_instant,
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

async fn handle_control_message_stream<S>(torrents: Rc<RefCell<TorrentMaps>>, mut stream: S)
where
    S: futures_lite::Stream<Item = SwarmControlMessage> + ::std::marker::Unpin,
{
    while let Some(message) = stream.next().await {
        match message {
            SwarmControlMessage::ConnectionClosed {
                info_hash,
                peer_id,
                ip_version,
            } => {
                ::log::debug!("Removing peer from torrents because connection was closed");

                if let IpVersion::V4 = ip_version {
                    if let Some(torrent_data) = torrents.borrow_mut().ipv4.get_mut(&info_hash) {
                        torrent_data.remove_peer(peer_id);
                    }
                } else {
                    if let Some(torrent_data) = torrents.borrow_mut().ipv6.get_mut(&info_hash) {
                        torrent_data.remove_peer(peer_id);
                    }
                }
            }
        }
    }
}

async fn handle_request_stream<S>(
    config: Config,
    torrents: Rc<RefCell<TorrentMaps>>,
    server_start_instant: ServerStartInstant,
    out_message_senders: Rc<Senders<(OutMessageMeta, OutMessage)>>,
    stream: S,
) where
    S: futures_lite::Stream<Item = (InMessageMeta, InMessage)> + ::std::marker::Unpin,
{
    let rng = Rc::new(RefCell::new(SmallRng::from_entropy()));

    let max_peer_age = config.cleaning.max_peer_age;
    let peer_valid_until = Rc::new(RefCell::new(ValidUntil::new(
        server_start_instant,
        max_peer_age,
    )));

    TimerActionRepeat::repeat(enclose!((peer_valid_until) move || {
        enclose!((peer_valid_until) move || async move {
            *peer_valid_until.borrow_mut() = ValidUntil::new(server_start_instant, max_peer_age);

            Some(Duration::from_secs(1))
        })()
    }));

    let config = &config;
    let torrents = &torrents;
    let peer_valid_until = &peer_valid_until;
    let rng = &rng;
    let out_message_senders = &out_message_senders;

    stream
        .for_each_concurrent(
            SHARED_IN_CHANNEL_SIZE,
            move |(meta, in_message)| async move {
                let mut out_messages = Vec::new();

                match in_message {
                    InMessage::AnnounceRequest(request) => handle_announce_request(
                        &config,
                        &mut rng.borrow_mut(),
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
                        .send_to(meta.out_message_consumer_id.0 as usize, (meta, out_message))
                        .await
                        .expect("failed sending out_message to socket worker");

                    ::log::debug!("swarm worker sent OutMessage to socket worker");
                }
            },
        )
        .await;
}

fn handle_announce_request(
    config: &Config,
    rng: &mut SmallRng,
    torrent_maps: &mut TorrentMaps,
    out_messages: &mut Vec<(OutMessageMeta, OutMessage)>,
    valid_until: ValidUntil,
    request_sender_meta: InMessageMeta,
    request: AnnounceRequest,
) {
    let (torrent_data, ip_version): (&mut TorrentData, &'static str) =
        if let IpVersion::V4 = request_sender_meta.ip_version {
            (torrent_maps.ipv4.entry(request.info_hash).or_default(), "4")
        } else {
            (torrent_maps.ipv6.entry(request.info_hash).or_default(), "6")
        };

    // If there is already a peer with this peer_id, check that connection id
    // is same as that of request sender. Otherwise, ignore request. Since
    // peers have access to each others peer_id's, they could send requests
    // using them, causing all sorts of issues.
    if let Some(previous_peer) = torrent_data.peers.get(&request.peer_id) {
        if request_sender_meta.connection_id != previous_peer.connection_id {
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

        let opt_removed_peer = match peer_status {
            PeerStatus::Leeching => {
                let peer = Peer {
                    connection_id: request_sender_meta.connection_id,
                    consumer_id: request_sender_meta.out_message_consumer_id,
                    seeder: false,
                    valid_until,
                };

                torrent_data.peers.insert(request.peer_id, peer)
            }
            PeerStatus::Seeding => {
                torrent_data.num_seeders += 1;

                let peer = Peer {
                    connection_id: request_sender_meta.connection_id,
                    consumer_id: request_sender_meta.out_message_consumer_id,
                    seeder: true,
                    valid_until,
                };

                torrent_data.peers.insert(request.peer_id, peer)
            }
            PeerStatus::Stopped => torrent_data.peers.remove(&request.peer_id),
        };

        if let Some(&Peer { seeder: true, .. }) = opt_removed_peer.as_ref() {
            torrent_data.num_seeders -= 1;
        }

        #[cfg(feature = "metrics")]
        match peer_status {
            PeerStatus::Stopped if opt_removed_peer.is_some() => {
                ::metrics::decrement_gauge!(
                    "aquatic_peers",
                    1.0,
                    "ip_version" => ip_version,
                    "worker_index" => WORKER_INDEX.with(|index| index.get()).to_string(),
                );
            }
            PeerStatus::Leeching | PeerStatus::Seeding if opt_removed_peer.is_none() => {
                ::metrics::increment_gauge!(
                    "aquatic_peers",
                    1.0,
                    "ip_version" => ip_version,
                    "worker_index" => WORKER_INDEX.with(|index| index.get()).to_string(),
                );
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
            let offer_out_message = OfferOutMessage {
                action: AnnounceAction::Announce,
                info_hash: request.info_hash,
                peer_id: request.peer_id,
                offer: offer.offer,
                offer_id: offer.offer_id,
            };

            let meta = OutMessageMeta {
                out_message_consumer_id: offer_receiver.consumer_id,
                connection_id: offer_receiver.connection_id,
                pending_scrape_id: None,
            };

            out_messages.push((meta, OutMessage::OfferOutMessage(offer_out_message)));
            ::log::trace!("sending middleman offer to {:?}", meta);
        }
    }

    // If peer sent answer, send it on to relevant peer
    if let (Some(answer), Some(answer_receiver_id), Some(offer_id)) = (
        request.answer,
        request.answer_to_peer_id,
        request.answer_offer_id,
    ) {
        if let Some(answer_receiver) = torrent_data.peers.get(&answer_receiver_id) {
            let answer_out_message = AnswerOutMessage {
                action: AnnounceAction::Announce,
                peer_id: request.peer_id,
                info_hash: request.info_hash,
                answer,
                offer_id,
            };

            let meta = OutMessageMeta {
                out_message_consumer_id: answer_receiver.consumer_id,
                connection_id: answer_receiver.connection_id,
                pending_scrape_id: None,
            };

            out_messages.push((meta, OutMessage::AnswerOutMessage(answer_out_message)));
            ::log::trace!("sending middleman answer to {:?}", meta);
        }
    }

    let out_message = OutMessage::AnnounceResponse(AnnounceResponse {
        action: AnnounceAction::Announce,
        info_hash: request.info_hash,
        complete: torrent_data.num_seeders,
        incomplete: torrent_data.num_leechers(),
        announce_interval: config.protocol.peer_announce_interval,
    });

    out_messages.push((request_sender_meta.into(), out_message));
}

fn handle_scrape_request(
    config: &Config,
    torrent_maps: &mut TorrentMaps,
    out_messages: &mut Vec<(OutMessageMeta, OutMessage)>,
    meta: InMessageMeta,
    request: ScrapeRequest,
) {
    let info_hashes = if let Some(info_hashes) = request.info_hashes {
        info_hashes.as_vec()
    } else {
        return;
    };

    let num_to_take = info_hashes.len().min(config.protocol.max_scrape_torrents);

    let mut out_message = ScrapeResponse {
        action: ScrapeAction::Scrape,
        files: HashMap::with_capacity(num_to_take),
    };

    let torrent_map: &mut TorrentMap = if let IpVersion::V4 = meta.ip_version {
        &mut torrent_maps.ipv4
    } else {
        &mut torrent_maps.ipv6
    };

    for info_hash in info_hashes.into_iter().take(num_to_take) {
        if let Some(torrent_data) = torrent_map.get(&info_hash) {
            let stats = ScrapeStatistics {
                complete: torrent_data.num_seeders,
                downloaded: 0, // No implementation planned
                incomplete: torrent_data.num_leechers(),
            };

            out_message.files.insert(info_hash, stats);
        }
    }

    out_messages.push((meta.into(), OutMessage::ScrapeResponse(out_message)));
}
