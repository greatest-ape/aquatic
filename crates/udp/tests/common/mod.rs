#![allow(dead_code)]

use std::{
    io::Cursor,
    net::{SocketAddr, UdpSocket},
    num::NonZeroU16,
    time::Duration,
};

use anyhow::Context;
use aquatic_udp::{common::BUFFER_SIZE, config::Config};
use aquatic_udp_protocol::{
    common::PeerId, AnnounceEvent, AnnounceRequest, ConnectRequest, ConnectionId, InfoHash,
    Ipv4AddrBytes, NumberOfBytes, NumberOfPeers, PeerKey, Port, Request, Response, ScrapeRequest,
    ScrapeResponse, TransactionId,
};

// FIXME: should ideally try different ports and use sync primitives to find
// out if tracker was successfully started
pub fn run_tracker(config: Config) {
    ::std::thread::spawn(move || {
        aquatic_udp::run(config).unwrap();
    });

    ::std::thread::sleep(Duration::from_secs(1));
}

pub fn connect(socket: &UdpSocket, tracker_addr: SocketAddr) -> anyhow::Result<ConnectionId> {
    let request = Request::Connect(ConnectRequest {
        transaction_id: TransactionId::new(0),
    });

    let response = request_and_response(socket, tracker_addr, request)?;

    if let Response::Connect(response) = response {
        Ok(response.connection_id)
    } else {
        Err(anyhow::anyhow!("not connect response: {:?}", response))
    }
}

pub fn announce(
    socket: &UdpSocket,
    tracker_addr: SocketAddr,
    connection_id: ConnectionId,
    peer_port: NonZeroU16,
    info_hash: InfoHash,
    peers_wanted: usize,
    seeder: bool,
) -> anyhow::Result<Response> {
    let mut peer_id = PeerId([0; 20]);

    for chunk in peer_id.0.chunks_exact_mut(2) {
        chunk.copy_from_slice(&peer_port.get().to_ne_bytes());
    }

    let request = Request::Announce(AnnounceRequest {
        connection_id,
        action_placeholder: Default::default(),
        transaction_id: TransactionId::new(0),
        info_hash,
        peer_id,
        bytes_downloaded: NumberOfBytes::new(0),
        bytes_uploaded: NumberOfBytes::new(0),
        bytes_left: NumberOfBytes::new(if seeder { 0 } else { 1 }),
        event: AnnounceEvent::Started.into(),
        ip_address: Ipv4AddrBytes([0; 4]),
        key: PeerKey::new(0),
        peers_wanted: NumberOfPeers::new(peers_wanted as i32),
        port: Port::new(peer_port),
    });

    request_and_response(socket, tracker_addr, request)
}

pub fn scrape(
    socket: &UdpSocket,
    tracker_addr: SocketAddr,
    connection_id: ConnectionId,
    info_hashes: Vec<InfoHash>,
) -> anyhow::Result<ScrapeResponse> {
    let request = Request::Scrape(ScrapeRequest {
        connection_id,
        transaction_id: TransactionId::new(0),
        info_hashes,
    });

    let response = request_and_response(socket, tracker_addr, request)?;

    if let Response::Scrape(response) = response {
        Ok(response)
    } else {
        Err(anyhow::anyhow!("not scrape response: {:?}", response))
    }
}

pub fn request_and_response(
    socket: &UdpSocket,
    tracker_addr: SocketAddr,
    request: Request,
) -> anyhow::Result<Response> {
    let mut buffer = [0u8; BUFFER_SIZE];

    {
        let mut buffer = Cursor::new(&mut buffer[..]);

        request
            .write_bytes(&mut buffer)
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

        Response::parse_bytes(&buffer[..bytes_read], true).with_context(|| "parse response")
    }
}
