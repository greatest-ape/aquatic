mod common;

use common::*;

use std::{
    io::{Cursor, ErrorKind},
    net::{Ipv4Addr, SocketAddr, SocketAddrV4, UdpSocket},
    time::Duration,
};

use anyhow::Context;
use aquatic_udp::{common::BUFFER_SIZE, config::Config};
use aquatic_udp_protocol::{
    common::PeerId, AnnounceEvent, AnnounceRequest, ConnectionId, InfoHash, NumberOfBytes,
    NumberOfPeers, PeerKey, Port, Request, TransactionId,
};

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
