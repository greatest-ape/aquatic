mod common;

use common::*;

use std::{
    io::{Cursor, ErrorKind},
    net::{Ipv4Addr, SocketAddr, SocketAddrV4, UdpSocket},
    num::NonZeroU16,
    time::Duration,
};

use anyhow::Context;
use aquatic_udp::{common::BUFFER_SIZE, config::Config};
use aquatic_udp_protocol::{
    common::PeerId, AnnounceEvent, AnnounceRequest, ConnectionId, InfoHash, Ipv4AddrBytes,
    NumberOfBytes, NumberOfPeers, PeerKey, Port, Request, ScrapeRequest, TransactionId,
};

#[test]
fn test_invalid_connection_id() -> anyhow::Result<()> {
    const TRACKER_PORT: u16 = 40_112;

    let mut config = Config::default();

    config.network.address_ipv4.set_port(TRACKER_PORT);
    config.network.use_ipv6 = false;

    run_tracker(config);

    let tracker_addr = SocketAddr::V4(SocketAddrV4::new(Ipv4Addr::LOCALHOST, TRACKER_PORT));
    let peer_addr = SocketAddr::V4(SocketAddrV4::new(Ipv4Addr::LOCALHOST, 0));

    let socket = UdpSocket::bind(peer_addr)?;

    socket.set_read_timeout(Some(Duration::from_secs(1)))?;

    // Send connect request to make sure that the tracker in fact responds to
    // valid requests
    let connection_id = connect(&socket, tracker_addr).with_context(|| "connect")?;

    let invalid_connection_id = ConnectionId(!connection_id.0);

    let announce_request = Request::Announce(AnnounceRequest {
        connection_id: invalid_connection_id,
        action_placeholder: Default::default(),
        transaction_id: TransactionId::new(0),
        info_hash: InfoHash([0; 20]),
        peer_id: PeerId([0; 20]),
        bytes_downloaded: NumberOfBytes::new(0),
        bytes_uploaded: NumberOfBytes::new(0),
        bytes_left: NumberOfBytes::new(0),
        event: AnnounceEvent::Started.into(),
        ip_address: Ipv4AddrBytes([0; 4]),
        key: PeerKey::new(0),
        peers_wanted: NumberOfPeers::new(10),
        port: Port::new(NonZeroU16::new(1).unwrap()),
    });

    let scrape_request = Request::Scrape(ScrapeRequest {
        connection_id: invalid_connection_id,
        transaction_id: TransactionId::new(0),
        info_hashes: vec![InfoHash([0; 20])],
    });

    no_response(&socket, tracker_addr, announce_request).with_context(|| "announce")?;
    no_response(&socket, tracker_addr, scrape_request).with_context(|| "scrape")?;

    Ok(())
}

fn no_response(
    socket: &UdpSocket,
    tracker_addr: SocketAddr,
    request: Request,
) -> anyhow::Result<()> {
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

    match socket.recv_from(&mut buffer) {
        Ok(_) => Err(anyhow::anyhow!("received response")),
        Err(err) if err.kind() == ErrorKind::WouldBlock => Ok(()),
        Err(err) => Err(err.into()),
    }
}
