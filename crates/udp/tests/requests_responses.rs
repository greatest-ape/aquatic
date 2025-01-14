mod common;

use common::*;

use std::{
    collections::{hash_map::RandomState, HashSet},
    net::{Ipv4Addr, SocketAddr, SocketAddrV4, UdpSocket},
    num::NonZeroU16,
    time::Duration,
};

use anyhow::Context;
use aquatic_udp::config::Config;
use aquatic_udp_protocol::{InfoHash, Response};

#[test]
fn test_multiple_connect_announce_scrape() -> anyhow::Result<()> {
    const TRACKER_PORT: u16 = 40_111;
    const PEER_PORT_START: u16 = 30_000;
    const PEERS_WANTED: usize = 10;

    let mut config = Config::default();

    config.network.address_ipv4.set_port(TRACKER_PORT);
    config.network.use_ipv6 = false;

    run_tracker(config);

    let tracker_addr = SocketAddr::V4(SocketAddrV4::new(Ipv4Addr::LOCALHOST, TRACKER_PORT));
    let peer_addr = SocketAddr::V4(SocketAddrV4::new(Ipv4Addr::LOCALHOST, 0));

    let info_hash = InfoHash([0; 20]);

    let mut num_seeders = 0;
    let mut num_leechers = 0;

    for i in 0..20 {
        let is_seeder = i % 3 == 0;

        let socket = UdpSocket::bind(peer_addr)?;
        socket.set_read_timeout(Some(Duration::from_secs(1)))?;

        let connection_id = connect(&socket, tracker_addr).with_context(|| "connect")?;

        let announce_response = {
            let response = announce(
                &socket,
                tracker_addr,
                connection_id,
                NonZeroU16::new(PEER_PORT_START + i as u16).unwrap(),
                info_hash,
                PEERS_WANTED,
                is_seeder,
            )
            .with_context(|| "announce")?;

            if let Response::AnnounceIpv4(response) = response {
                response
            } else {
                return Err(anyhow::anyhow!("not announce response: {:?}", response));
            }
        };

        assert_eq!(announce_response.peers.len(), i.min(PEERS_WANTED));

        assert_eq!(announce_response.fixed.seeders.0.get(), num_seeders);
        assert_eq!(announce_response.fixed.leechers.0.get(), num_leechers);

        let response_peer_ports: HashSet<u16, RandomState> =
            HashSet::from_iter(announce_response.peers.iter().map(|p| p.port.0.get()));
        let expected_peer_ports: HashSet<u16, RandomState> =
            HashSet::from_iter((0..i).map(|i| PEER_PORT_START + i as u16));

        if i > PEERS_WANTED {
            assert!(response_peer_ports.is_subset(&expected_peer_ports));
        } else {
            assert_eq!(response_peer_ports, expected_peer_ports);
        }

        // Do this after announce is evaluated, since it is expected not to include announcing peer
        if is_seeder {
            num_seeders += 1;
        } else {
            num_leechers += 1;
        }

        let scrape_response = scrape(
            &socket,
            tracker_addr,
            connection_id,
            vec![info_hash, InfoHash([1; 20])],
        )
        .with_context(|| "scrape")?;

        assert_eq!(
            scrape_response.torrent_stats[0].seeders.0.get(),
            num_seeders
        );
        assert_eq!(
            scrape_response.torrent_stats[0].leechers.0.get(),
            num_leechers
        );
        assert_eq!(scrape_response.torrent_stats[1].seeders.0.get(), 0);
        assert_eq!(scrape_response.torrent_stats[1].leechers.0.get(), 0);
    }

    Ok(())
}
