mod common;

use std::cell::RefCell;
use std::net::{IpAddr, Ipv4Addr, Ipv6Addr};
use std::rc::Rc;

use aquatic_http_protocol::request::AnnounceRequest;
use rand::prelude::SmallRng;
use rand::SeedableRng;
use tokio::sync::mpsc::Receiver;
use tokio::task::LocalSet;
use tokio::time;

use aquatic_common::{extract_response_peers, CanonicalSocketAddr, ValidUntil};
use aquatic_http_protocol::response::{
    AnnounceResponse, Response, ResponsePeer, ResponsePeerListV4, ResponsePeerListV6,
};

use crate::common::ChannelAnnounceRequest;
use crate::config::Config;

use common::*;

pub fn run_request_worker(
    config: Config,
    request_receiver: Receiver<ChannelAnnounceRequest>,
) -> anyhow::Result<()> {
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()?;

    runtime.block_on(run_inner(config, request_receiver))?;

    Ok(())
}

async fn run_inner(
    config: Config,
    mut request_receiver: Receiver<ChannelAnnounceRequest>,
) -> anyhow::Result<()> {
    let torrents = Rc::new(RefCell::new(TorrentMaps::default()));
    let mut rng = SmallRng::from_entropy();

    LocalSet::new().spawn_local(periodically_clean_torrents(
        config.clone(),
        torrents.clone(),
    ));

    loop {
        let request = request_receiver
            .recv()
            .await
            .ok_or_else(|| anyhow::anyhow!("request channel closed"))?;

        let valid_until = ValidUntil::new(config.cleaning.max_peer_age);

        let response = handle_announce_request(
            &config,
            &mut rng,
            &mut torrents.borrow_mut(),
            valid_until,
            request.source_addr,
            request.request.into(),
        );

        let _ = request.response_sender.send(Response::Announce(response));
    }
}

async fn periodically_clean_torrents(config: Config, torrents: Rc<RefCell<TorrentMaps>>) {
    let mut interval = time::interval(time::Duration::from_secs(
        config.cleaning.torrent_cleaning_interval,
    ));

    loop {
        interval.tick().await;

        torrents.borrow_mut().clean();
    }
}

fn handle_announce_request(
    config: &Config,
    rng: &mut SmallRng,
    torrent_maps: &mut TorrentMaps,
    valid_until: ValidUntil,
    source_addr: CanonicalSocketAddr,
    request: AnnounceRequest,
) -> AnnounceResponse {
    match source_addr.get().ip() {
        IpAddr::V4(source_ip) => {
            let torrent_data: &mut TorrentData<Ipv4Addr> =
                torrent_maps.ipv4.entry(request.info_hash).or_default();

            let (seeders, leechers, response_peers) = upsert_peer_and_get_response_peers(
                config,
                rng,
                torrent_data,
                source_ip,
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
        IpAddr::V6(source_ip) => {
            let torrent_data: &mut TorrentData<Ipv6Addr> =
                torrent_maps.ipv6.entry(request.info_hash).or_default();

            let (seeders, leechers, response_peers) = upsert_peer_and_get_response_peers(
                config,
                rng,
                torrent_data,
                source_ip,
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
    rng: &mut SmallRng,
    torrent_data: &mut TorrentData<I>,
    source_ip: I,
    request: AnnounceRequest,
    valid_until: ValidUntil,
) -> (usize, usize, Vec<ResponsePeer<I>>) {
    // Insert/update/remove peer who sent this request

    let peer_status =
        PeerStatus::from_event_and_bytes_left(request.event, Some(request.bytes_left));

    let peer = Peer {
        ip_address: source_ip,
        port: request.port,
        status: peer_status,
        valid_until,
    };

    let peer_map_key = PeerMapKey {
        peer_id: request.peer_id,
        ip_address: source_ip,
    };

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

    match opt_removed_peer.map(|peer| peer.status) {
        Some(PeerStatus::Leeching) => {
            torrent_data.num_leechers -= 1;
        }
        Some(PeerStatus::Seeding) => {
            torrent_data.num_seeders -= 1;
        }
        _ => {}
    }

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
