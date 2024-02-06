use std::sync::Arc;

use aquatic_common::access_list::{create_access_list_cache, AccessListArcSwap, AccessListCache};
use aquatic_ws_protocol::incoming::{
    AnnounceEvent, AnnounceRequest, AnnounceRequestOffer, ScrapeRequest,
};
use aquatic_ws_protocol::outgoing::{
    AnnounceResponse, AnswerOutMessage, ErrorResponse, ErrorResponseAction, OfferOutMessage,
    OutMessage, ScrapeResponse, ScrapeStatistics,
};
use hashbrown::HashMap;
use rand::rngs::SmallRng;

use aquatic_common::{IndexMap, SecondsSinceServerStart, ServerStartInstant};
use aquatic_ws_protocol::common::*;
use rand::Rng;

use crate::common::*;
use crate::config::Config;

pub struct TorrentMaps {
    ipv4: TorrentMap,
    ipv6: TorrentMap,
}

impl TorrentMaps {
    pub fn new(worker_index: usize) -> Self {
        Self {
            ipv4: TorrentMap::new(worker_index, IpVersion::V4),
            ipv6: TorrentMap::new(worker_index, IpVersion::V6),
        }
    }

    pub fn handle_announce_request(
        &mut self,
        config: &Config,
        rng: &mut SmallRng,
        out_messages: &mut Vec<(OutMessageMeta, OutMessage)>,
        server_start_instant: ServerStartInstant,
        request_sender_meta: InMessageMeta,
        request: AnnounceRequest,
    ) {
        let torrent_map = self.get_torrent_map_by_ip_version(request_sender_meta.ip_version);

        torrent_map.handle_announce_request(
            config,
            rng,
            out_messages,
            server_start_instant,
            request_sender_meta,
            request,
        );
    }

    pub fn handle_scrape_request(
        &mut self,
        config: &Config,
        out_messages: &mut Vec<(OutMessageMeta, OutMessage)>,
        meta: InMessageMeta,
        request: ScrapeRequest,
    ) {
        let torrent_map = self.get_torrent_map_by_ip_version(meta.ip_version);

        torrent_map.handle_scrape_request(config, out_messages, meta, request);
    }

    pub fn clean(
        &mut self,
        config: &Config,
        access_list: &Arc<AccessListArcSwap>,
        server_start_instant: ServerStartInstant,
    ) {
        let mut access_list_cache = create_access_list_cache(access_list);
        let now = server_start_instant.seconds_elapsed();

        self.ipv4.clean(config, &mut access_list_cache, now);
        self.ipv6.clean(config, &mut access_list_cache, now);
    }

    #[cfg(feature = "metrics")]
    pub fn update_torrent_count_metrics(&self) {
        self.ipv4.update_torrent_gauge();
        self.ipv6.update_torrent_gauge();
    }

    pub fn handle_connection_closed(
        &mut self,
        info_hash: InfoHash,
        peer_id: PeerId,
        ip_version: IpVersion,
    ) {
        let torrent_map = self.get_torrent_map_by_ip_version(ip_version);

        torrent_map.handle_connection_closed(info_hash, peer_id);
    }

    fn get_torrent_map_by_ip_version(&mut self, ip_version: IpVersion) -> &mut TorrentMap {
        match ip_version {
            IpVersion::V4 => &mut self.ipv4,
            IpVersion::V6 => &mut self.ipv6,
        }
    }
}

struct TorrentMap {
    torrents: IndexMap<InfoHash, TorrentData>,
    #[cfg(feature = "metrics")]
    torrent_gauge: ::metrics::Gauge,
    #[cfg(feature = "metrics")]
    peer_gauge: ::metrics::Gauge,
}

impl TorrentMap {
    pub fn new(worker_index: usize, ip_version: IpVersion) -> Self {
        #[cfg(feature = "metrics")]
        let peer_gauge = match ip_version {
            IpVersion::V4 => ::metrics::gauge!(
                "aquatic_peers",
                "ip_version" => "4",
                "worker_index" => worker_index.to_string(),
            ),
            IpVersion::V6 => ::metrics::gauge!(
                "aquatic_peers",
                "ip_version" => "6",
                "worker_index" => worker_index.to_string(),
            ),
        };
        #[cfg(feature = "metrics")]
        let torrent_gauge = match ip_version {
            IpVersion::V4 => ::metrics::gauge!(
                "aquatic_torrents",
                "ip_version" => "4",
                "worker_index" => worker_index.to_string(),
            ),
            IpVersion::V6 => ::metrics::gauge!(
                "aquatic_torrents",
                "ip_version" => "6",
                "worker_index" => worker_index.to_string(),
            ),
        };

        Self {
            torrents: Default::default(),
            #[cfg(feature = "metrics")]
            peer_gauge,
            #[cfg(feature = "metrics")]
            torrent_gauge,
        }
    }

    pub fn handle_announce_request(
        &mut self,
        config: &Config,
        rng: &mut SmallRng,
        out_messages: &mut Vec<(OutMessageMeta, OutMessage)>,
        server_start_instant: ServerStartInstant,
        request_sender_meta: InMessageMeta,
        request: AnnounceRequest,
    ) {
        let torrent_data = self.torrents.entry(request.info_hash).or_default();

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

        let peer_status = torrent_data.insert_or_update_peer(
            config,
            server_start_instant,
            request_sender_meta,
            &request,
            #[cfg(feature = "metrics")]
            &self.peer_gauge,
        );

        if peer_status != PeerStatus::Stopped {
            if let Some(offers) = request.offers {
                torrent_data.handle_offers(
                    config,
                    rng,
                    server_start_instant,
                    request.info_hash,
                    request.peer_id,
                    offers,
                    out_messages,
                );
            }

            if let (Some(answer), Some(answer_receiver_id), Some(offer_id)) = (
                request.answer,
                request.answer_to_peer_id,
                request.answer_offer_id,
            ) {
                let opt_out_message = torrent_data.handle_answer(
                    request_sender_meta,
                    request.info_hash,
                    request.peer_id,
                    answer_receiver_id,
                    offer_id,
                    answer,
                );

                if let Some(out_message) = opt_out_message {
                    out_messages.push(out_message);
                }
            }
        }

        let response = OutMessage::AnnounceResponse(AnnounceResponse {
            action: AnnounceAction::Announce,
            info_hash: request.info_hash,
            complete: torrent_data.num_seeders,
            incomplete: torrent_data.num_leechers(),
            announce_interval: config.protocol.peer_announce_interval,
        });

        out_messages.push((request_sender_meta.into(), response));
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

        for info_hash in info_hashes.into_iter().take(num_to_take) {
            if let Some(torrent_data) = self.torrents.get(&info_hash) {
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

    pub fn handle_connection_closed(&mut self, info_hash: InfoHash, peer_id: PeerId) {
        if let Some(torrent_data) = self.torrents.get_mut(&info_hash) {
            torrent_data.handle_connection_closed(
                peer_id,
                #[cfg(feature = "metrics")]
                &self.peer_gauge,
            );
        }
    }

    #[cfg(feature = "metrics")]
    pub fn update_torrent_gauge(&self) {
        self.torrent_gauge.set(self.torrents.len() as f64);
    }

    fn clean(
        &mut self,
        config: &Config,
        access_list_cache: &mut AccessListCache,
        now: SecondsSinceServerStart,
    ) {
        let mut total_num_peers = 0u64;

        self.torrents.retain(|info_hash, torrent_data| {
            if !access_list_cache
                .load()
                .allows(config.access_list.mode, &info_hash.0)
            {
                return false;
            }

            let num_peers = torrent_data.clean_and_get_num_peers(now);

            total_num_peers += num_peers as u64;

            num_peers > 0
        });

        self.torrents.shrink_to_fit();

        #[cfg(feature = "metrics")]
        self.peer_gauge.set(total_num_peers as f64);

        #[cfg(feature = "metrics")]
        self.update_torrent_gauge();
    }
}

#[derive(Default)]
struct TorrentData {
    peers: IndexMap<PeerId, Peer>,
    num_seeders: usize,
}

impl TorrentData {
    fn num_leechers(&self) -> usize {
        self.peers.len() - self.num_seeders
    }

    pub fn insert_or_update_peer(
        &mut self,
        config: &Config,
        server_start_instant: ServerStartInstant,
        request_sender_meta: InMessageMeta,
        request: &AnnounceRequest,
        #[cfg(feature = "metrics")] peer_gauge: &::metrics::Gauge,
    ) -> PeerStatus {
        let valid_until = ValidUntil::new(server_start_instant, config.cleaning.max_peer_age);

        let peer_status = PeerStatus::from_event_and_bytes_left(
            request.event.unwrap_or_default(),
            request.bytes_left,
        );

        match self.peers.entry(request.peer_id) {
            ::indexmap::map::Entry::Occupied(mut entry) => match peer_status {
                PeerStatus::Leeching => {
                    let peer = entry.get_mut();

                    if peer.seeder {
                        self.num_seeders -= 1;
                    }

                    peer.seeder = false;
                    peer.valid_until = valid_until;
                }
                PeerStatus::Seeding => {
                    let peer = entry.get_mut();

                    if !peer.seeder {
                        self.num_seeders += 1;
                    }

                    peer.seeder = true;
                    peer.valid_until = valid_until;
                }
                PeerStatus::Stopped => {
                    let peer = entry.swap_remove();

                    if peer.seeder {
                        self.num_seeders -= 1;
                    }

                    #[cfg(feature = "metrics")]
                    peer_gauge.decrement(1.0);
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
                    peer_gauge.increment(1.0)
                }
                PeerStatus::Seeding => {
                    self.num_seeders += 1;

                    let peer = Peer {
                        connection_id: request_sender_meta.connection_id,
                        consumer_id: request_sender_meta.out_message_consumer_id,
                        seeder: true,
                        valid_until,
                        expecting_answers: Default::default(),
                    };

                    entry.insert(peer);

                    #[cfg(feature = "metrics")]
                    peer_gauge.increment(1.0);
                }
                PeerStatus::Stopped => (),
            },
        }

        peer_status
    }

    /// Pass on offers to random peers
    #[allow(clippy::too_many_arguments)]
    pub fn handle_offers(
        &mut self,
        config: &Config,
        rng: &mut SmallRng,
        server_start_instant: ServerStartInstant,
        info_hash: InfoHash,
        sender_peer_id: PeerId,
        offers: Vec<AnnounceRequestOffer>,
        out_messages: &mut Vec<(OutMessageMeta, OutMessage)>,
    ) {
        let max_num_peers_to_take = offers.len().min(config.protocol.max_offers);

        let offer_receivers: Vec<(PeerId, ConnectionId, ConsumerId)> = extract_response_peers(
            rng,
            &self.peers,
            max_num_peers_to_take,
            sender_peer_id,
            |peer_id, peer| (*peer_id, peer.connection_id, peer.consumer_id),
        );

        if let Some(peer) = self.peers.get_mut(&sender_peer_id) {
            for (
                offer,
                (offer_receiver_peer_id, offer_receiver_connection_id, offer_receiver_consumer_id),
            ) in offers.into_iter().zip(offer_receivers)
            {
                peer.expecting_answers.insert(
                    ExpectingAnswer {
                        from_peer_id: offer_receiver_peer_id,
                        regarding_offer_id: offer.offer_id,
                    },
                    ValidUntil::new(server_start_instant, config.cleaning.max_offer_age),
                );

                let offer_out_message = OfferOutMessage {
                    action: AnnounceAction::Announce,
                    info_hash,
                    peer_id: sender_peer_id,
                    offer: offer.offer,
                    offer_id: offer.offer_id,
                };

                let meta = OutMessageMeta {
                    out_message_consumer_id: offer_receiver_consumer_id,
                    connection_id: offer_receiver_connection_id,
                    pending_scrape_id: None,
                };

                out_messages.push((meta, OutMessage::OfferOutMessage(offer_out_message)));
            }
        }
    }

    /// Pass on answer to relevant peer
    fn handle_answer(
        &mut self,
        request_sender_meta: InMessageMeta,
        info_hash: InfoHash,
        peer_id: PeerId,
        answer_receiver_id: PeerId,
        offer_id: OfferId,
        answer: RtcAnswer,
    ) -> Option<(OutMessageMeta, OutMessage)> {
        if let Some(answer_receiver) = self.peers.get_mut(&answer_receiver_id) {
            let expecting_answer = ExpectingAnswer {
                from_peer_id: peer_id,
                regarding_offer_id: offer_id,
            };

            if answer_receiver
                .expecting_answers
                .swap_remove(&expecting_answer)
                .is_some()
            {
                let answer_out_message = AnswerOutMessage {
                    action: AnnounceAction::Announce,
                    peer_id,
                    info_hash,
                    answer,
                    offer_id,
                };

                let meta = OutMessageMeta {
                    out_message_consumer_id: answer_receiver.consumer_id,
                    connection_id: answer_receiver.connection_id,
                    pending_scrape_id: None,
                };

                Some((meta, OutMessage::AnswerOutMessage(answer_out_message)))
            } else {
                let error_message = ErrorResponse {
                    action: Some(ErrorResponseAction::Announce),
                    info_hash: Some(info_hash),
                    failure_reason:
                        "Could not find the offer corresponding to your answer. It may have expired."
                            .into(),
                };

                Some((
                    request_sender_meta.into(),
                    OutMessage::ErrorResponse(error_message),
                ))
            }
        } else {
            None
        }
    }

    pub fn handle_connection_closed(
        &mut self,
        peer_id: PeerId,
        #[cfg(feature = "metrics")] peer_gauge: &::metrics::Gauge,
    ) {
        if let Some(peer) = self.peers.swap_remove(&peer_id) {
            if peer.seeder {
                self.num_seeders -= 1;
            }

            #[cfg(feature = "metrics")]
            peer_gauge.decrement(1.0);
        }
    }

    fn clean_and_get_num_peers(&mut self, now: SecondsSinceServerStart) -> usize {
        self.peers.retain(|_, peer| {
            peer.expecting_answers
                .retain(|_, valid_until| valid_until.valid(now));
            peer.expecting_answers.shrink_to_fit();

            let keep = peer.valid_until.valid(now);

            if (!keep) & peer.seeder {
                self.num_seeders -= 1;
            }

            keep
        });

        self.peers.shrink_to_fit();

        self.peers.len()
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

/// Extract response peers
///
/// If there are more peers in map than `max_num_peers_to_take`, do a random
/// selection of peers from first and second halves of map in order to avoid
/// returning too homogeneous peers.
///
/// Filters out announcing peer.
#[inline]
pub fn extract_response_peers<K, V, R, F>(
    rng: &mut impl Rng,
    peer_map: &IndexMap<K, V>,
    max_num_peers_to_take: usize,
    sender_peer_map_key: K,
    peer_conversion_function: F,
) -> Vec<R>
where
    K: Eq + ::std::hash::Hash,
    F: Fn(&K, &V) -> R,
{
    if peer_map.len() <= max_num_peers_to_take + 1 {
        // This branch: number of peers in map (minus sender peer) is less than
        // or equal to number of peers to take, so return all except sender
        // peer.
        let mut peers = Vec::with_capacity(peer_map.len());

        peers.extend(peer_map.iter().filter_map(|(k, v)| {
            (*k != sender_peer_map_key).then_some(peer_conversion_function(k, v))
        }));

        // Handle the case when sender peer is not in peer list. Typically,
        // this function will not be called when this is the case.
        if peers.len() > max_num_peers_to_take {
            peers.pop();
        }

        peers
    } else {
        // Note: if this branch is taken, the peer map contains at least two
        // more peers than max_num_peers_to_take

        let middle_index = peer_map.len() / 2;
        // Add one to take two extra peers in case sender peer is among
        // selected peers and will need to be filtered out
        let num_to_take_per_half = (max_num_peers_to_take / 2) + 1;

        let offset_half_one = {
            let from = 0;
            let to = usize::max(1, middle_index - num_to_take_per_half);

            rng.gen_range(from..to)
        };
        let offset_half_two = {
            let from = middle_index;
            let to = usize::max(middle_index + 1, peer_map.len() - num_to_take_per_half);

            rng.gen_range(from..to)
        };

        let end_half_one = offset_half_one + num_to_take_per_half;
        let end_half_two = offset_half_two + num_to_take_per_half;

        let mut peers = Vec::with_capacity(max_num_peers_to_take + 2);

        if let Some(slice) = peer_map.get_range(offset_half_one..end_half_one) {
            peers.extend(slice.iter().filter_map(|(k, v)| {
                (*k != sender_peer_map_key).then_some(peer_conversion_function(k, v))
            }));
        }
        if let Some(slice) = peer_map.get_range(offset_half_two..end_half_two) {
            peers.extend(slice.iter().filter_map(|(k, v)| {
                (*k != sender_peer_map_key).then_some(peer_conversion_function(k, v))
            }));
        }

        while peers.len() > max_num_peers_to_take {
            peers.pop();
        }

        peers
    }
}

#[cfg(test)]
mod tests {
    use hashbrown::HashSet;
    use rand::{rngs::SmallRng, SeedableRng};

    use super::*;

    #[test]
    fn test_extract_response_peers() {
        let mut rng = SmallRng::from_entropy();

        for num_peers_in_map in 0..50 {
            for max_num_peers_to_take in 0..50 {
                for sender_peer_map_key in 0..50 {
                    test_extract_response_peers_helper(
                        &mut rng,
                        num_peers_in_map,
                        max_num_peers_to_take,
                        sender_peer_map_key,
                    );
                }
            }
        }
    }

    fn test_extract_response_peers_helper(
        rng: &mut SmallRng,
        num_peers_in_map: usize,
        max_num_peers_to_take: usize,
        sender_peer_map_key: usize,
    ) {
        let peer_map = IndexMap::from_iter((0..num_peers_in_map).map(|i| (i, i)));

        let response_peers = extract_response_peers(
            rng,
            &peer_map,
            max_num_peers_to_take,
            sender_peer_map_key,
            |_, p| *p,
        );

        if num_peers_in_map > max_num_peers_to_take + 1 {
            assert_eq!(response_peers.len(), max_num_peers_to_take);
        } else {
            assert!(response_peers.len() <= max_num_peers_to_take);
        }

        assert!(!response_peers.contains(&sender_peer_map_key));

        let unique: HashSet<_> = response_peers.iter().copied().collect();

        assert_eq!(response_peers.len(), unique.len(),);
    }
}
