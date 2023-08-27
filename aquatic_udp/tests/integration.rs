use std::{
    collections::{hash_map::RandomState, HashSet},
    io::Cursor,
    net::{Ipv4Addr, SocketAddr, SocketAddrV4, UdpSocket},
    time::Duration,
};

use anyhow::Context;
use aquatic_udp::{common::BUFFER_SIZE, config::Config};
use aquatic_udp_protocol::{
    common::PeerId, AnnounceEvent, AnnounceRequest, AnnounceResponse, ConnectRequest, InfoHash,
    NumberOfBytes, NumberOfPeers, PeerKey, Port, Request, Response, TransactionId,
};

const PEER_PORT_START: u16 = 30_000;
const TRACKER_PORT: u16 = 40_111;
const PEERS_WANTED: usize = 10;

#[test]
fn test_multiple_connect_and_announce() {
    let config = Config::default();

    let tracker_port = run_tracker(config);

    let info_hash = InfoHash([0; 20]);

    let mut num_seeders = 0;
    let mut num_leechers = 0;

    for i in 0..20 {
        let is_seeder = i % 3 == 0;

        if is_seeder {
            num_seeders += 1;
        } else {
            num_leechers += 1;
        }

        let response = match connect_and_announce(
            tracker_port,
            PEER_PORT_START + i as u16,
            info_hash,
            is_seeder,
        ) {
            Ok(response) => response,
            Err(err) => {
                panic!("connect_and_announce failed: {:#}", err);
            }
        };

        assert_eq!(response.peers.len(), i.min(PEERS_WANTED));

        assert_eq!(response.seeders.0, num_seeders);
        assert_eq!(response.leechers.0, num_leechers);

        let response_peer_ports: HashSet<u16, RandomState> =
            HashSet::from_iter(response.peers.iter().map(|p| p.port.0));
        let expected_peer_ports: HashSet<u16, RandomState> =
            HashSet::from_iter((0..i).map(|i| PEER_PORT_START + i as u16));

        if i > PEERS_WANTED {
            assert!(response_peer_ports.is_subset(&expected_peer_ports));
        } else {
            assert_eq!(response_peer_ports, expected_peer_ports);
        }
    }
}

// FIXME: should ideally try different ports and use sync primitives to find
// out if tracker was successfully started
fn run_tracker(mut config: Config) -> u16 {
    let port = TRACKER_PORT;

    config.network.address.set_port(port);

    ::std::thread::spawn(move || {
        aquatic_udp::run(config).unwrap();
    });

    ::std::thread::sleep(Duration::from_secs(1));

    port
}

fn connect_and_announce(
    tracker_port: u16,
    peer_port: u16,
    info_hash: InfoHash,
    seeder: bool,
) -> anyhow::Result<AnnounceResponse<Ipv4Addr>> {
    let peer_addr = SocketAddr::V4(SocketAddrV4::new(Ipv4Addr::LOCALHOST, 0));
    let tracker_addr = SocketAddr::V4(SocketAddrV4::new(Ipv4Addr::LOCALHOST, tracker_port));

    let socket = UdpSocket::bind(peer_addr)?;
    let mut buffer = [0u8; BUFFER_SIZE];

    socket.set_read_timeout(Some(Duration::from_secs(1)))?;

    {
        let mut buffer = Cursor::new(&mut buffer[..]);

        Request::Connect(ConnectRequest {
            transaction_id: TransactionId(0),
        })
        .write(&mut buffer)
        .with_context(|| "write connect request")?;

        let bytes_written = buffer.position() as usize;

        socket
            .send_to(&(buffer.into_inner())[..bytes_written], tracker_addr)
            .with_context(|| "send connect request")?;
    }

    let connection_id = {
        let (bytes_read, _) = socket
            .recv_from(&mut buffer)
            .with_context(|| "recv connect response")?;

        if let Response::Connect(response) = Response::from_bytes(&buffer[..bytes_read], true)
            .with_context(|| "parse connect response")?
        {
            response.connection_id
        } else {
            panic!("Not connect response");
        }
    };

    {
        let mut buffer = Cursor::new(&mut buffer[..]);

        let mut peer_id = PeerId([0; 20]);

        for chunk in peer_id.0.chunks_exact_mut(2) {
            chunk.copy_from_slice(&peer_port.to_ne_bytes());
        }

        Request::Announce(AnnounceRequest {
            connection_id,
            transaction_id: TransactionId(0),
            info_hash,
            peer_id,
            bytes_downloaded: NumberOfBytes(0),
            bytes_uploaded: NumberOfBytes(0),
            bytes_left: NumberOfBytes(if seeder { 0 } else { 1 }),
            event: AnnounceEvent::Started,
            ip_address: None,
            key: PeerKey(0),
            peers_wanted: NumberOfPeers(PEERS_WANTED as i32),
            port: Port(peer_port),
        })
        .write(&mut buffer)
        .with_context(|| "write announce request")?;

        let bytes_written = buffer.position() as usize;

        socket
            .send_to(&(buffer.into_inner())[..bytes_written], tracker_addr)
            .with_context(|| "send announce request")?;
    }

    {
        let (bytes_read, _) = socket
            .recv_from(&mut buffer)
            .with_context(|| "recv announce response")?;

        if let Response::AnnounceIpv4(response) = Response::from_bytes(&buffer[..bytes_read], true)
            .with_context(|| "parse announce response")?
        {
            Ok(response)
        } else {
            panic!("Not announce response");
        }
    }
}
