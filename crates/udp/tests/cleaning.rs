mod common;

use aquatic_common::{CanonicalSocketAddr, ServerStartInstant, ValidUntil};
use crossbeam_channel::unbounded;
use rand::make_rng;

use std::{
    net::{Ipv4Addr, SocketAddr, SocketAddrV4},
    sync::atomic::Ordering,
};

use aquatic_udp::{
    common::{State, Statistics},
    config::Config,
    swarm::TorrentMaps,
};
use aquatic_udp_protocol::{
    common::PeerId, AnnounceEvent, AnnounceRequest, AnnounceResponse, ConnectionId, InfoHash,
    Ipv4AddrBytes, NumberOfBytes, NumberOfPeers, PeerKey, Port, Response, TransactionId,
};

#[test]
fn test_cleaning() -> anyhow::Result<()> {
    const NUM_TORRENTS: u8 = 2;
    const NUM_PEERS: u8 = 50;

    let mut config = Config::default();

    config.protocol.max_response_peers = 100;
    config.statistics.print_to_stdout = true; // Just to enable calculating statistics

    let state = State::default();
    let statistics = Statistics::new(&config);
    let torrent_maps = TorrentMaps::default();
    let mut rng = make_rng();

    let (statistics_sender, _statistics_receiver) = unbounded();

    let server_start_instant = ServerStartInstant::new();

    let short = ValidUntil::new(server_start_instant, 0);
    let long = ValidUntil::new(server_start_instant, 4);

    for i in 0..NUM_TORRENTS {
        for j in 0..NUM_PEERS {
            let (request, src) = make_request_and_src(j, i);

            let valid_until = if j % 2 == 0 { short } else { long };

            let response = torrent_maps.announce(
                &config,
                &statistics_sender,
                &mut rng,
                &request,
                src,
                valid_until,
            );

            match response {
                Response::AnnounceIpv4(AnnounceResponse { fixed: _, peers }) => {
                    assert_eq!(peers.len(), j as usize);
                }
                _ => panic!("Wrong response type"),
            }
        }
    }

    // Clean out half

    ::std::thread::sleep(::std::time::Duration::from_secs(1));

    let elapsed = server_start_instant.seconds_elapsed().get();
    assert!(elapsed > 0 && elapsed < 4);

    torrent_maps.clean_and_update_statistics(
        &config,
        &statistics.swarm.clone(),
        &statistics_sender,
        &state.access_list,
        server_start_instant,
    );

    assert_eq!(
        statistics.swarm.ipv4.peers.load(Ordering::SeqCst),
        (NUM_PEERS as usize * NUM_TORRENTS as usize) / 2
    );
    assert_eq!(
        statistics.swarm.ipv4.torrents.load(Ordering::SeqCst),
        NUM_TORRENTS as usize
    );

    for i in 0..NUM_TORRENTS {
        let (request, src) = make_request_and_src(NUM_PEERS, i);

        let response =
            torrent_maps.announce(&config, &statistics_sender, &mut rng, &request, src, short);

        match response {
            Response::AnnounceIpv4(AnnounceResponse { fixed: _, peers }) => {
                assert_eq!(peers.len(), NUM_PEERS as usize / 2);
            }
            _ => panic!("Wrong response type"),
        }
    }

    // Clean out rest

    ::std::thread::sleep(::std::time::Duration::from_secs(4));

    let elapsed = server_start_instant.seconds_elapsed().get();
    assert!(elapsed > 4);

    torrent_maps.clean_and_update_statistics(
        &config,
        &statistics.swarm.clone(),
        &statistics_sender,
        &state.access_list,
        server_start_instant,
    );

    assert_eq!(statistics.swarm.ipv4.peers.load(Ordering::SeqCst), 0);
    assert_eq!(statistics.swarm.ipv4.torrents.load(Ordering::SeqCst), 0);

    for i in 0..NUM_TORRENTS {
        let (request, src) = make_request_and_src(NUM_PEERS, i);

        let response =
            torrent_maps.announce(&config, &statistics_sender, &mut rng, &request, src, short);

        match response {
            Response::AnnounceIpv4(AnnounceResponse { fixed: _, peers }) => {
                assert_eq!(peers.len(), 0);
            }
            _ => panic!("Wrong response type"),
        }
    }

    Ok(())
}

fn make_request_and_src(i: u8, torrent: u8) -> (AnnounceRequest, CanonicalSocketAddr) {
    let request = AnnounceRequest {
        connection_id: ConnectionId::new(0),
        action_placeholder: Default::default(),
        transaction_id: TransactionId::new(0),
        info_hash: InfoHash([torrent; 20]),
        peer_id: PeerId([i; 20]),
        bytes_downloaded: NumberOfBytes::new(0),
        bytes_uploaded: NumberOfBytes::new(0),
        bytes_left: NumberOfBytes::new(1),
        event: AnnounceEvent::Started,
        ip_address: Ipv4AddrBytes([0; 4]),
        key: PeerKey::new(0),
        peers_wanted: NumberOfPeers::new(0),
        port: Port::new(1.try_into().unwrap()),
    };

    let src = CanonicalSocketAddr::new(SocketAddr::V4(SocketAddrV4::new(
        Ipv4Addr::new(127, 0, 0, i + 1),
        1024,
    )));

    (request, src)
}
