use std::cell::RefCell;
use std::collections::BTreeMap;
use std::net::{IpAddr, Ipv4Addr, Ipv6Addr};
use std::rc::Rc;
use std::sync::Arc;
use std::time::Duration;
use std::time::Instant;

use either::Either;
use futures_lite::{Stream, StreamExt};
use glommio::channels::channel_mesh::{MeshBuilder, Partial, Role, Senders};
use glommio::timer::TimerActionRepeat;
use glommio::{enclose, prelude::*};
use rand::prelude::SmallRng;
use rand::Rng;
use rand::SeedableRng;
use smartstring::{LazyCompact, SmartString};

use aquatic_common::access_list::{create_access_list_cache, AccessListArcSwap, AccessListCache};
use aquatic_common::ValidUntil;
use aquatic_common::{extract_response_peers, PanicSentinel};
use aquatic_common::{AmortizedIndexMap, CanonicalSocketAddr};
use aquatic_http_protocol::common::*;
use aquatic_http_protocol::request::*;
use aquatic_http_protocol::response::ResponsePeer;
use aquatic_http_protocol::response::*;

use crate::common::*;
use crate::config::Config;

pub trait Ip: ::std::fmt::Debug + Copy + Eq + ::std::hash::Hash {}

impl Ip for Ipv4Addr {}
impl Ip for Ipv6Addr {}

#[derive(Clone, Copy, Debug)]
pub struct ConnectionMeta {
    /// Index of socket worker responsible for this connection. Required for
    /// sending back response through correct channel to correct worker.
    pub response_consumer_id: ConsumerId,
    pub peer_addr: CanonicalSocketAddr,
    /// Connection id local to socket worker
    pub connection_id: ConnectionId,
}

#[derive(Clone, Copy, Debug)]
pub struct PeerConnectionMeta<I: Ip> {
    pub response_consumer_id: ConsumerId,
    pub connection_id: ConnectionId,
    pub peer_ip_address: I,
}

#[derive(PartialEq, Eq, Clone, Copy, Debug)]
pub enum PeerStatus {
    Seeding,
    Leeching,
    Stopped,
}

impl PeerStatus {
    /// Determine peer status from announce event and number of bytes left.
    ///
    /// Likely, the last branch will be taken most of the time.
    #[inline]
    pub fn from_event_and_bytes_left(event: AnnounceEvent, opt_bytes_left: Option<usize>) -> Self {
        if let AnnounceEvent::Stopped = event {
            Self::Stopped
        } else if let Some(0) = opt_bytes_left {
            Self::Seeding
        } else {
            Self::Leeching
        }
    }
}

#[derive(Debug, Clone, Copy)]
pub struct Peer<I: Ip> {
    pub connection_meta: PeerConnectionMeta<I>,
    pub port: u16,
    pub status: PeerStatus,
    pub valid_until: ValidUntil,
}

impl<I: Ip> Peer<I> {
    pub fn to_response_peer(&self) -> ResponsePeer<I> {
        ResponsePeer {
            ip_address: self.connection_meta.peer_ip_address,
            port: self.port,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct PeerMapKey<I: Ip> {
    pub peer_id: PeerId,
    pub ip_or_key: Either<I, SmartString<LazyCompact>>,
}

pub type PeerMap<I> = AmortizedIndexMap<PeerMapKey<I>, Peer<I>>;

pub struct TorrentData<I: Ip> {
    pub peers: PeerMap<I>,
    pub num_seeders: usize,
    pub num_leechers: usize,
}

impl<I: Ip> Default for TorrentData<I> {
    #[inline]
    fn default() -> Self {
        Self {
            peers: Default::default(),
            num_seeders: 0,
            num_leechers: 0,
        }
    }
}

pub type TorrentMap<I> = AmortizedIndexMap<InfoHash, TorrentData<I>>;

#[derive(Default)]
pub struct TorrentMaps {
    pub ipv4: TorrentMap<Ipv4Addr>,
    pub ipv6: TorrentMap<Ipv6Addr>,
}

impl TorrentMaps {
    pub fn clean(&mut self, config: &Config, access_list: &Arc<AccessListArcSwap>) {
        let mut access_list_cache = create_access_list_cache(access_list);

        Self::clean_torrent_map(config, &mut access_list_cache, &mut self.ipv4);
        Self::clean_torrent_map(config, &mut access_list_cache, &mut self.ipv6);
    }

    fn clean_torrent_map<I: Ip>(
        config: &Config,
        access_list_cache: &mut AccessListCache,
        torrent_map: &mut TorrentMap<I>,
    ) {
        let now = Instant::now();

        torrent_map.retain(|info_hash, torrent_data| {
            if !access_list_cache
                .load()
                .allows(config.access_list.mode, &info_hash.0)
            {
                return false;
            }

            let num_seeders = &mut torrent_data.num_seeders;
            let num_leechers = &mut torrent_data.num_leechers;

            torrent_data.peers.retain(|_, peer| {
                let keep = peer.valid_until.0 >= now;

                if !keep {
                    match peer.status {
                        PeerStatus::Seeding => {
                            *num_seeders -= 1;
                        }
                        PeerStatus::Leeching => {
                            *num_leechers -= 1;
                        }
                        _ => (),
                    };
                }

                keep
            });

            !torrent_data.peers.is_empty()
        });

        torrent_map.shrink_to_fit();
    }
}

pub async fn run_request_worker(
    _sentinel: PanicSentinel,
    config: Config,
    state: State,
    request_mesh_builder: MeshBuilder<ChannelRequest, Partial>,
    response_mesh_builder: MeshBuilder<ChannelResponse, Partial>,
) {
    let (_, mut request_receivers) = request_mesh_builder.join(Role::Consumer).await.unwrap();
    let (response_senders, _) = response_mesh_builder.join(Role::Producer).await.unwrap();

    let response_senders = Rc::new(response_senders);

    let torrents = Rc::new(RefCell::new(TorrentMaps::default()));
    let access_list = state.access_list;

    // Periodically clean torrents
    TimerActionRepeat::repeat(enclose!((config, torrents, access_list) move || {
        enclose!((config, torrents, access_list) move || async move {
            torrents.borrow_mut().clean(&config, &access_list);

            Some(Duration::from_secs(config.cleaning.torrent_cleaning_interval))
        })()
    }));

    let mut handles = Vec::new();

    for (_, receiver) in request_receivers.streams() {
        let handle = spawn_local(handle_request_stream(
            config.clone(),
            torrents.clone(),
            response_senders.clone(),
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
    response_senders: Rc<Senders<ChannelResponse>>,
    mut stream: S,
) where
    S: Stream<Item = ChannelRequest> + ::std::marker::Unpin,
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

    while let Some(channel_request) = stream.next().await {
        let (response, consumer_id) = match channel_request {
            ChannelRequest::Announce {
                request,
                peer_addr,
                response_consumer_id,
                connection_id,
            } => {
                let meta = ConnectionMeta {
                    response_consumer_id,
                    connection_id,
                    peer_addr,
                };

                let response = handle_announce_request(
                    &config,
                    &mut rng,
                    &mut torrents.borrow_mut(),
                    peer_valid_until.borrow().to_owned(),
                    meta,
                    request,
                );

                let response = ChannelResponse::Announce {
                    response,
                    peer_addr,
                    connection_id,
                };

                (response, response_consumer_id)
            }
            ChannelRequest::Scrape {
                request,
                peer_addr,
                response_consumer_id,
                connection_id,
            } => {
                let meta = ConnectionMeta {
                    response_consumer_id,
                    connection_id,
                    peer_addr,
                };

                let response =
                    handle_scrape_request(&config, &mut torrents.borrow_mut(), meta, request);

                let response = ChannelResponse::Scrape {
                    response,
                    peer_addr,
                    connection_id,
                };

                (response, response_consumer_id)
            }
        };

        ::log::debug!("preparing to send response to channel: {:?}", response);

        if let Err(err) = response_senders.try_send_to(consumer_id.0, response) {
            ::log::warn!("response_sender.try_send: {:?}", err);
        }

        yield_if_needed().await;
    }
}

pub fn handle_announce_request(
    config: &Config,
    rng: &mut impl Rng,
    torrent_maps: &mut TorrentMaps,
    valid_until: ValidUntil,
    meta: ConnectionMeta,
    request: AnnounceRequest,
) -> AnnounceResponse {
    match meta.peer_addr.get().ip() {
        IpAddr::V4(peer_ip_address) => {
            let torrent_data: &mut TorrentData<Ipv4Addr> =
                torrent_maps.ipv4.entry(request.info_hash).or_default();

            let peer_connection_meta = PeerConnectionMeta {
                response_consumer_id: meta.response_consumer_id,
                connection_id: meta.connection_id,
                peer_ip_address,
            };

            let (seeders, leechers, response_peers) = upsert_peer_and_get_response_peers(
                config,
                rng,
                peer_connection_meta,
                torrent_data,
                request,
                valid_until,
            );

            let response = AnnounceResponse {
                complete: seeders,
                incomplete: leechers,
                announce_interval: config.protocol.peer_announce_interval,
                peers: ResponsePeerListV4(response_peers),
                peers6: ResponsePeerListV6(vec![]),
                warning_message: None,
            };

            response
        }
        IpAddr::V6(peer_ip_address) => {
            let torrent_data: &mut TorrentData<Ipv6Addr> =
                torrent_maps.ipv6.entry(request.info_hash).or_default();

            let peer_connection_meta = PeerConnectionMeta {
                response_consumer_id: meta.response_consumer_id,
                connection_id: meta.connection_id,
                peer_ip_address,
            };

            let (seeders, leechers, response_peers) = upsert_peer_and_get_response_peers(
                config,
                rng,
                peer_connection_meta,
                torrent_data,
                request,
                valid_until,
            );

            let response = AnnounceResponse {
                complete: seeders,
                incomplete: leechers,
                announce_interval: config.protocol.peer_announce_interval,
                peers: ResponsePeerListV4(vec![]),
                peers6: ResponsePeerListV6(response_peers),
                warning_message: None,
            };

            response
        }
    }
}

/// Insert/update peer. Return num_seeders, num_leechers and response peers
pub fn upsert_peer_and_get_response_peers<I: Ip>(
    config: &Config,
    rng: &mut impl Rng,
    request_sender_meta: PeerConnectionMeta<I>,
    torrent_data: &mut TorrentData<I>,
    request: AnnounceRequest,
    valid_until: ValidUntil,
) -> (usize, usize, Vec<ResponsePeer<I>>) {
    // Insert/update/remove peer who sent this request

    let peer_status =
        PeerStatus::from_event_and_bytes_left(request.event, Some(request.bytes_left));

    let peer = Peer {
        connection_meta: request_sender_meta,
        port: request.port,
        status: peer_status,
        valid_until,
    };

    ::log::debug!("peer: {:?}", peer);

    let ip_or_key = request
        .key
        .map(Either::Right)
        .unwrap_or_else(|| Either::Left(request_sender_meta.peer_ip_address));

    let peer_map_key = PeerMapKey {
        peer_id: request.peer_id,
        ip_or_key,
    };

    ::log::debug!("peer map key: {:?}", peer_map_key);

    let opt_removed_peer = match peer_status {
        PeerStatus::Leeching => {
            torrent_data.num_leechers += 1;

            torrent_data.peers.insert(peer_map_key.clone(), peer)
        }
        PeerStatus::Seeding => {
            torrent_data.num_seeders += 1;

            torrent_data.peers.insert(peer_map_key.clone(), peer)
        }
        PeerStatus::Stopped => torrent_data.peers.remove(&peer_map_key),
    };

    ::log::debug!("opt_removed_peer: {:?}", opt_removed_peer);

    match opt_removed_peer.map(|peer| peer.status) {
        Some(PeerStatus::Leeching) => {
            torrent_data.num_leechers -= 1;
        }
        Some(PeerStatus::Seeding) => {
            torrent_data.num_seeders -= 1;
        }
        _ => {}
    }

    ::log::debug!("peer request numwant: {:?}", request.numwant);

    let max_num_peers_to_take = match request.numwant {
        Some(0) | None => config.protocol.max_peers,
        Some(numwant) => numwant.min(config.protocol.max_peers),
    };

    let response_peers: Vec<ResponsePeer<I>> = extract_response_peers(
        rng,
        &torrent_data.peers,
        max_num_peers_to_take,
        peer_map_key,
        Peer::to_response_peer,
    );

    (
        torrent_data.num_seeders,
        torrent_data.num_leechers,
        response_peers,
    )
}

pub fn handle_scrape_request(
    config: &Config,
    torrent_maps: &mut TorrentMaps,
    meta: ConnectionMeta,
    request: ScrapeRequest,
) -> ScrapeResponse {
    let num_to_take = request
        .info_hashes
        .len()
        .min(config.protocol.max_scrape_torrents);

    let mut response = ScrapeResponse {
        files: BTreeMap::new(),
    };

    let peer_ip = meta.peer_addr.get().ip();

    // If request.info_hashes is empty, don't return scrape for all
    // torrents, even though reference server does it. It is too expensive.
    if peer_ip.is_ipv4() {
        for info_hash in request.info_hashes.into_iter().take(num_to_take) {
            if let Some(torrent_data) = torrent_maps.ipv4.get(&info_hash) {
                let stats = ScrapeStatistics {
                    complete: torrent_data.num_seeders,
                    downloaded: 0, // No implementation planned
                    incomplete: torrent_data.num_leechers,
                };

                response.files.insert(info_hash, stats);
            }
        }
    } else {
        for info_hash in request.info_hashes.into_iter().take(num_to_take) {
            if let Some(torrent_data) = torrent_maps.ipv6.get(&info_hash) {
                let stats = ScrapeStatistics {
                    complete: torrent_data.num_seeders,
                    downloaded: 0, // No implementation planned
                    incomplete: torrent_data.num_leechers,
                };

                response.files.insert(info_hash, stats);
            }
        }
    };

    response
}
