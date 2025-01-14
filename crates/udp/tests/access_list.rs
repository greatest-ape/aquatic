mod common;

use common::*;

use std::{
    fs::File,
    io::Write,
    net::{Ipv4Addr, SocketAddr, SocketAddrV4, UdpSocket},
    num::NonZeroU16,
    time::Duration,
};

use anyhow::Context;
use aquatic_common::access_list::AccessListMode;
use aquatic_udp::config::Config;
use aquatic_udp_protocol::{InfoHash, Response};

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

    config.network.address_ipv4.set_port(tracker_port);
    config.network.use_ipv6 = false;

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
        NonZeroU16::new(1).unwrap(),
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
        NonZeroU16::new(1).unwrap(),
        info_hash_success,
        10,
        false,
    )
    .with_context(|| "announce")?;

    assert!(matches!(response, Response::AnnounceIpv4(_)));

    Ok(())
}
