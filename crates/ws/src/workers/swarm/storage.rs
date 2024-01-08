use std::sync::Arc;

use aquatic_common::access_list::{create_access_list_cache, AccessListArcSwap, AccessListCache};
use aquatic_ws_protocol::incoming::{AnnounceEvent, AnnounceRequest, ScrapeRequest};
use aquatic_ws_protocol::outgoing::{
    AnnounceResponse, AnswerOutMessage, ErrorResponse, ErrorResponseAction, OfferOutMessage,
    OutMessage, ScrapeResponse, ScrapeStatistics,
};
use hashbrown::HashMap;
use metrics::Gauge;
use rand::rngs::SmallRng;

use aquatic_common::{
    extract_response_peers, IndexMap, SecondsSinceServerStart, ServerStartInstant,
};
use aquatic_ws_protocol::common::*;

use crate::common::*;
use crate::config::Config;
use crate::workers::swarm::WORKER_INDEX;

type TorrentMap = IndexMap<InfoHash, TorrentData>;
type PeerMap = IndexMap<PeerId, Peer>;

pub struct TorrentMaps {
    ipv4: TorrentMap,
    ipv6: TorrentMap,
    peers_gauge_ipv4: Gauge,
    peers_gauge_ipv6: Gauge,
    torrents_gauge_ipv4: Gauge,
    torrents_gauge_ipv6: Gauge,
}

impl Default for TorrentMaps {
    fn default() -> Self {
        Self {
            ipv4: Default::default(),
            ipv6: Default::default(),
            peers_gauge_ipv4: ::metrics::gauge!(
                "aquatic_peers",
                "ip_version" => "4",
                "worker_index" => WORKER_INDEX.with(|index| index.get()).to_string(),
            ),
            peers_gauge_ipv6: ::metrics::gauge!(
                "aquatic_peers",
                "ip_version" => "6",
                "worker_index" => WORKER_INDEX.with(|index| index.get()).to_string(),
            ),
            torrents_gauge_ipv4: ::metrics::gauge!(
                "aquatic_torrents",
                "ip_version" => "4",
                "worker_index" => WORKER_INDEX.with(|index| index.get()).to_string(),
            ),
            torrents_gauge_ipv6: ::metrics::gauge!(
                "aquatic_torrents",
                "ip_version" => "6",
                "worker_index" => WORKER_INDEX.with(|index| index.get()).to_string(),
            ),
        }
    }
}

impl TorrentMaps {
    pub fn handle_announce_request(
        &mut self,
        config: &Config,
        rng: &mut SmallRng,
        out_messages: &mut Vec<(OutMessageMeta, OutMessage)>,
        server_start_instant: ServerStartInstant,
        request_sender_meta: InMessageMeta,
        request: AnnounceRequest,
    ) {
        let torrent_data = if let IpVersion::V4 = request_sender_meta.ip_version {
            self.ipv4.entry(request.info_hash).or_default()
        } else {
            self.ipv6.entry(request.info_hash).or_default()
        };

        let valid_until = ValidUntil::new(server_start_instant, config.cleaning.max_peer_age);

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

            match torrent_data.peers.entry(request.peer_id) {
                ::indexmap::map::Entry::Occupied(mut entry) => match peer_status {
                    PeerStatus::Leeching => {
                        let peer = entry.get_mut();

                        if peer.seeder {
                            torrent_data.num_seeders -= 1;
                        }

                        peer.seeder = false;
                        peer.valid_until = valid_until;
                    }
                    PeerStatus::Seeding => {
                        let peer = entry.get_mut();

                        if !peer.seeder {
                            torrent_data.num_seeders += 1;
                        }

                        peer.seeder = true;
                        peer.valid_until = valid_until;
                    }
                    PeerStatus::Stopped => {
                        let peer = entry.remove();

                        if peer.seeder {
                            torrent_data.num_seeders -= 1;
                        }

                        #[cfg(feature = "metrics")]
                        match request_sender_meta.ip_version {
                            IpVersion::V4 => self.peers_gauge_ipv4.decrement(1.0),
                            IpVersion::V6 => self.peers_gauge_ipv6.decrement(1.0),
                        }

                        return;
                    }
                },
                ::indexmap::map::Entry::Vacant(entry) => match peer_status {
                    PeerStatus::Leeching => {
                        let peer = Peer {
                            connection_id: request_sender_meta.connection_id,
                            consumer_id: request_sender_meta.out_message_consumer_id,
                            seeder: false,
                            valid_until,
                            expecting_answers: Default::default(),
                        };

                        entry.insert(peer);

                        #[cfg(feature = "metrics")]
                        match request_sender_meta.ip_version {
                            IpVersion::V4 => self.peers_gauge_ipv4.increment(1.0),
                            IpVersion::V6 => self.peers_gauge_ipv6.increment(1.0),
                        }
                    }
                    PeerStatus::Seeding => {
                        torrent_data.num_seeders += 1;

                        let peer = Peer {
                            connection_id: request_sender_meta.connection_id,
                            consumer_id: request_sender_meta.out_message_consumer_id,
                            seeder: true,
                            valid_until,
                            expecting_answers: Default::default(),
                        };

                        entry.insert(peer);

                        #[cfg(feature = "metrics")]
                        match request_sender_meta.ip_version {
                            IpVersion::V4 => self.peers_gauge_ipv4.increment(1.0),
                            IpVersion::V6 => self.peers_gauge_ipv6.increment(1.0),
                        }
                    }
                    PeerStatus::Stopped => return,
                },
            }
        };

        // If peer sent offers, send them on to random peers
        if let Some(offers) = request.offers {
            // FIXME: config: also maybe check this when parsing request
            let max_num_peers_to_take = offers.len().min(config.protocol.max_offers);

            #[inline]
            fn convert_offer_receiver_peer(
                peer_id: &PeerId,
                peer: &Peer,
            ) -> (PeerId, ConnectionId, ConsumerId) {
                (*peer_id, peer.connection_id, peer.consumer_id)
            }

            let offer_receivers: Vec<(PeerId, ConnectionId, ConsumerId)> = extract_response_peers(
                rng,
                &torrent_data.peers,
                max_num_peers_to_take,
                request.peer_id,
                convert_offer_receiver_peer,
            );

            if let Some(peer) = torrent_data.peers.get_mut(&request.peer_id) {
                for (
                    offer,
                    (
                        offer_receiver_peer_id,
                        offer_receiver_connection_id,
                        offer_receiver_consumer_id,
                    ),
                ) in offers.into_iter().zip(offer_receivers)
                {
                    let offer_out_message = OfferOutMessage {
                        action: AnnounceAction::Announce,
                        info_hash: request.info_hash,
                        peer_id: request.peer_id,
                        offer: offer.offer,
                        offer_id: offer.offer_id,
                    };

                    let meta = OutMessageMeta {
                        out_message_consumer_id: offer_receiver_consumer_id,
                        connection_id: offer_receiver_connection_id,
                        pending_scrape_id: None,
                    };

                    out_messages.push((meta, OutMessage::OfferOutMessage(offer_out_message)));
                    ::log::trace!("sending OfferOutMessage to {:?}", meta);

                    peer.expecting_answers.insert(
                        ExpectingAnswer {
                            from_peer_id: offer_receiver_peer_id,
                            regarding_offer_id: offer.offer_id,
                        },
                        ValidUntil::new(server_start_instant, config.cleaning.max_offer_age),
                    );
                }
            }
        }

        // If peer sent answer, send it on to relevant peer
        if let (Some(answer), Some(answer_receiver_id), Some(offer_id)) = (
            request.answer,
            request.answer_to_peer_id,
            request.answer_offer_id,
        ) {
            if let Some(answer_receiver) = torrent_data.peers.get_mut(&answer_receiver_id) {
                let expecting_answer = ExpectingAnswer {
                    from_peer_id: request.peer_id,
                    regarding_offer_id: offer_id,
                };

                if let Some(_) = answer_receiver.expecting_answers.remove(&expecting_answer) {
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
                    ::log::trace!("sending AnswerOutMessage to {:?}", meta);
                } else {
                    let error_message = ErrorResponse {
                        action: Some(ErrorResponseAction::Announce),
                        info_hash: Some(request.info_hash),
                        failure_reason:
                            "Could not find the offer corresponding to your answer. It may have expired."
                                .into(),
                    };

                    out_messages.push((
                        request_sender_meta.into(),
                        OutMessage::ErrorResponse(error_message),
                    ));
                }
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

    pub fn handle_scrape_request(
        &mut self,
        config: &Config,
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
            &mut self.ipv4
        } else {
            &mut self.ipv6
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

    pub fn clean(
        &mut self,
        config: &Config,
        access_list: &Arc<AccessListArcSwap>,
        server_start_instant: ServerStartInstant,
    ) {
        let mut access_list_cache = create_access_list_cache(access_list);
        let now = server_start_instant.seconds_elapsed();

        Self::clean_torrent_map(
            config,
            &mut access_list_cache,
            &mut self.ipv4,
            now,
            &self.peers_gauge_ipv4,
        );
        Self::clean_torrent_map(
            config,
            &mut access_list_cache,
            &mut self.ipv6,
            now,
            &self.peers_gauge_ipv6,
        );
    }

    fn clean_torrent_map(
        config: &Config,
        access_list_cache: &mut AccessListCache,
        torrent_map: &mut TorrentMap,
        now: SecondsSinceServerStart,
        peers_gauge: &Gauge,
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
                peer.expecting_answers
                    .retain(|_, valid_until| valid_until.valid(now));
                peer.expecting_answers.shrink_to_fit();

                let keep = peer.valid_until.valid(now);

                if (!keep) & peer.seeder {
                    *num_seeders -= 1;
                }

                keep
            });

            total_num_peers += torrent_data.peers.len() as u64;

            torrent_data.peers.shrink_to_fit();

            !torrent_data.peers.is_empty()
        });

        torrent_map.shrink_to_fit();

        #[cfg(feature = "metrics")]
        peers_gauge.set(total_num_peers as f64)
    }

    #[cfg(feature = "metrics")]
    pub fn update_torrent_count_metrics(&self) {
        self.torrents_gauge_ipv4.set(self.ipv4.len() as f64);
        self.torrents_gauge_ipv6.set(self.ipv6.len() as f64);
    }

    pub fn handle_connection_closed(
        &mut self,
        info_hash: InfoHash,
        peer_id: PeerId,
        ip_version: IpVersion,
    ) {
        ::log::debug!("Removing peer from torrents because connection was closed");

        if let IpVersion::V4 = ip_version {
            if let Some(torrent_data) = self.ipv4.get_mut(&info_hash) {
                torrent_data.remove_peer(peer_id);

                #[cfg(feature = "metrics")]
                self.peers_gauge_ipv4.decrement(1.0);
            }
        } else {
            if let Some(torrent_data) = self.ipv6.get_mut(&info_hash) {
                torrent_data.remove_peer(peer_id);

                #[cfg(feature = "metrics")]
                self.peers_gauge_ipv6.decrement(1.0);
            }
        }
    }
}

struct TorrentData {
    peers: PeerMap,
    num_seeders: usize,
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
    fn remove_peer(&mut self, peer_id: PeerId) {
        if let Some(peer) = self.peers.remove(&peer_id) {
            if peer.seeder {
                self.num_seeders -= 1;
            }
        }
    }

    fn num_leechers(&self) -> usize {
        self.peers.len() - self.num_seeders
    }
}

#[derive(Clone, Debug)]
struct Peer {
    pub consumer_id: ConsumerId,
    pub connection_id: ConnectionId,
    pub seeder: bool,
    pub valid_until: ValidUntil,
    pub expecting_answers: IndexMap<ExpectingAnswer, ValidUntil>,
}

#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct ExpectingAnswer {
    pub from_peer_id: PeerId,
    pub regarding_offer_id: OfferId,
}

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
