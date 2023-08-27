use std::{
    collections::{hash_map::RandomState, HashSet},
    fs::File,
    io::{Cursor, ErrorKind, Write},
    net::{Ipv4Addr, SocketAddr, SocketAddrV4, UdpSocket},
    time::Duration,
};

use anyhow::Context;
use aquatic_common::access_list::AccessListMode;
use aquatic_udp::{common::BUFFER_SIZE, config::Config};
use aquatic_udp_protocol::{
    common::PeerId, AnnounceEvent, AnnounceRequest, ConnectRequest, ConnectionId, InfoHash,
    NumberOfBytes, NumberOfPeers, PeerKey, Port, Request, Response, ScrapeRequest, ScrapeResponse,
    TransactionId,
};

#[test]
fn test_multiple_connect_announce_scrape() -> anyhow::Result<()> {
    const TRACKER_PORT: u16 = 40_111;
    const PEER_PORT_START: u16 = 30_000;
    const PEERS_WANTED: usize = 10;

    let mut config = Config::default();

    config.network.address.set_port(TRACKER_PORT);

    run_tracker(config);

    let tracker_addr = SocketAddr::V4(SocketAddrV4::new(Ipv4Addr::LOCALHOST, TRACKER_PORT));
    let peer_addr = SocketAddr::V4(SocketAddrV4::new(Ipv4Addr::LOCALHOST, 0));

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

        let socket = UdpSocket::bind(peer_addr)?;
        socket.set_read_timeout(Some(Duration::from_secs(1)))?;

        let connection_id = connect(&socket, tracker_addr).with_context(|| "connect")?;

        let announce_response = {
            let response = announce(
                &socket,
                tracker_addr,
                connection_id,
                PEER_PORT_START + i as u16,
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

        assert_eq!(announce_response.seeders.0, num_seeders);
        assert_eq!(announce_response.leechers.0, num_leechers);

        let response_peer_ports: HashSet<u16, RandomState> =
            HashSet::from_iter(announce_response.peers.iter().map(|p| p.port.0));
        let expected_peer_ports: HashSet<u16, RandomState> =
            HashSet::from_iter((0..i).map(|i| PEER_PORT_START + i as u16));

        if i > PEERS_WANTED {
            assert!(response_peer_ports.is_subset(&expected_peer_ports));
        } else {
            assert_eq!(response_peer_ports, expected_peer_ports);
        }

        let scrape_response = scrape(
            &socket,
            tracker_addr,
            connection_id,
            vec![info_hash, InfoHash([1; 20])],
        )
        .with_context(|| "scrape")?;

        assert_eq!(scrape_response.torrent_stats[0].seeders.0, num_seeders);
        assert_eq!(scrape_response.torrent_stats[0].leechers.0, num_leechers);
        assert_eq!(scrape_response.torrent_stats[1].seeders.0, 0);
        assert_eq!(scrape_response.torrent_stats[1].leechers.0, 0);
    }

    Ok(())
}

#[test]
fn test_announce_with_invalid_connection_id() -> anyhow::Result<()> {
    const TRACKER_PORT: u16 = 40_112;

    let mut config = Config::default();

    config.network.address.set_port(TRACKER_PORT);

    run_tracker(config);

    let tracker_addr = SocketAddr::V4(SocketAddrV4::new(Ipv4Addr::LOCALHOST, TRACKER_PORT));
    let peer_addr = SocketAddr::V4(SocketAddrV4::new(Ipv4Addr::LOCALHOST, 0));

    let socket = UdpSocket::bind(peer_addr)?;

    socket.set_read_timeout(Some(Duration::from_secs(1)))?;

    // Make sure that the tracker in fact responds to requests
    let connection_id = connect(&socket, tracker_addr).with_context(|| "connect")?;

    let mut buffer = [0u8; BUFFER_SIZE];

    {
        let mut buffer = Cursor::new(&mut buffer[..]);

        let request = Request::Announce(AnnounceRequest {
            connection_id: ConnectionId(!connection_id.0),
            transaction_id: TransactionId(0),
            info_hash: InfoHash([0; 20]),
            peer_id: PeerId([0; 20]),
            bytes_downloaded: NumberOfBytes(0),
            bytes_uploaded: NumberOfBytes(0),
            bytes_left: NumberOfBytes(0),
            event: AnnounceEvent::Started,
            ip_address: None,
            key: PeerKey(0),
            peers_wanted: NumberOfPeers(-1),
            port: Port(1),
        });

        request
            .write(&mut buffer)
            .with_context(|| "write request")?;

        let bytes_written = buffer.position() as usize;

        socket
            .send_to(&(buffer.into_inner())[..bytes_written], tracker_addr)
            .with_context(|| "send request")?;
    }

    match socket.recv_from(&mut buffer) {
        Ok(_) => Err(anyhow::anyhow!("received response")),
        Err(err) if err.kind() == ErrorKind::WouldBlock => Ok(()),
        Err(err) => Err(err.into()),
    }
}

#[test]
fn test_access_list_deny() -> anyhow::Result<()> {
    const TRACKER_PORT: u16 = 40_113;

    let deny = InfoHash([0; 20]);
    let allow = InfoHash([1; 20]);

    test_access_list(TRACKER_PORT, allow, deny, deny, AccessListMode::Deny)?;

    Ok(())
}

#[test]
fn test_access_list_allow() -> anyhow::Result<()> {
    const TRACKER_PORT: u16 = 40_114;

    let allow = InfoHash([0; 20]);
    let deny = InfoHash([1; 20]);

    test_access_list(TRACKER_PORT, allow, deny, allow, AccessListMode::Allow)?;

    Ok(())
}

fn test_access_list(
    tracker_port: u16,
    info_hash_success: InfoHash,
    info_hash_fail: InfoHash,
    info_hash_in_list: InfoHash,
    mode: AccessListMode,
) -> anyhow::Result<()> {
    let access_list_dir = tempfile::tempdir().with_context(|| "get temporary directory")?;
    let access_list_path = access_list_dir.path().join("access-list.txt");

    let mut access_list_file =
        File::create(&access_list_path).with_context(|| "create access list file")?;
    writeln!(
        access_list_file,
        "{}",
        hex::encode_upper(info_hash_in_list.0)
    )
    .with_context(|| "write to access list file")?;

    let mut config = Config::default();

    config.network.address.set_port(tracker_port);

    config.access_list.mode = mode;
    config.access_list.path = access_list_path;

    run_tracker(config);

    let tracker_addr = SocketAddr::V4(SocketAddrV4::new(Ipv4Addr::LOCALHOST, tracker_port));
    let peer_addr = SocketAddr::V4(SocketAddrV4::new(Ipv4Addr::LOCALHOST, 0));

    let socket = UdpSocket::bind(peer_addr)?;
    socket.set_read_timeout(Some(Duration::from_secs(1)))?;

    let connection_id = connect(&socket, tracker_addr).with_context(|| "connect")?;

    let response = announce(
        &socket,
        tracker_addr,
        connection_id,
        1,
        info_hash_fail,
        10,
        false,
    )
    .with_context(|| "announce")?;

    assert!(
        matches!(response, Response::Error(_)),
        "response should be error but is {:?}",
        response
    );

    let response = announce(
        &socket,
        tracker_addr,
        connection_id,
        1,
        info_hash_success,
        10,
        false,
    )
    .with_context(|| "announce")?;

    assert!(matches!(response, Response::AnnounceIpv4(_)));

    Ok(())
}

// FIXME: should ideally try different ports and use sync primitives to find
// out if tracker was successfully started
fn run_tracker(config: Config) {
    ::std::thread::spawn(move || {
        aquatic_udp::run(config).unwrap();
    });

    ::std::thread::sleep(Duration::from_secs(1));
}

fn connect(socket: &UdpSocket, tracker_addr: SocketAddr) -> anyhow::Result<ConnectionId> {
    let request = Request::Connect(ConnectRequest {
        transaction_id: TransactionId(0),
    });

    let response = request_and_response(&socket, tracker_addr, request)?;

    if let Response::Connect(response) = response {
        Ok(response.connection_id)
    } else {
        Err(anyhow::anyhow!("not connect response: {:?}", response))
    }
}

fn announce(
    socket: &UdpSocket,
    tracker_addr: SocketAddr,
    connection_id: ConnectionId,
    peer_port: u16,
    info_hash: InfoHash,
    peers_wanted: usize,
    seeder: bool,
) -> anyhow::Result<Response> {
    let mut peer_id = PeerId([0; 20]);

    for chunk in peer_id.0.chunks_exact_mut(2) {
        chunk.copy_from_slice(&peer_port.to_ne_bytes());
    }

    let request = Request::Announce(AnnounceRequest {
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
        peers_wanted: NumberOfPeers(peers_wanted as i32),
        port: Port(peer_port),
    });

    Ok(request_and_response(&socket, tracker_addr, request)?)
}

fn scrape(
    socket: &UdpSocket,
    tracker_addr: SocketAddr,
    connection_id: ConnectionId,
    info_hashes: Vec<InfoHash>,
) -> anyhow::Result<ScrapeResponse> {
    let request = Request::Scrape(ScrapeRequest {
        connection_id,
        transaction_id: TransactionId(0),
        info_hashes,
    });

    let response = request_and_response(&socket, tracker_addr, request)?;

    if let Response::Scrape(response) = response {
        Ok(response)
    } else {
        return Err(anyhow::anyhow!("not scrape response: {:?}", response));
    }
}

fn request_and_response(
    socket: &UdpSocket,
    tracker_addr: SocketAddr,
    request: Request,
) -> anyhow::Result<Response> {
    let mut buffer = [0u8; BUFFER_SIZE];

    {
        let mut buffer = Cursor::new(&mut buffer[..]);

        request
            .write(&mut buffer)
            .with_context(|| "write request")?;

        let bytes_written = buffer.position() as usize;

        socket
            .send_to(&(buffer.into_inner())[..bytes_written], tracker_addr)
            .with_context(|| "send request")?;
    }

    {
        let (bytes_read, _) = socket
            .recv_from(&mut buffer)
            .with_context(|| "recv response")?;

        Ok(Response::from_bytes(&buffer[..bytes_read], true).with_context(|| "parse response")?)
    }
}
