use std::net::{IpAddr, SocketAddr};
use std::vec::Drain;

use parking_lot::MutexGuard;
use rand::rngs::SmallRng;

use aquatic_common::convert_ipv4_mapped_ipv6;
use aquatic_udp_protocol::*;

use crate::common::*;
use crate::common::announce::handle_announce_request;
use crate::config::Config;

#[inline]
pub fn handle_announce_requests(
    config: &Config,
    torrents: &mut MutexGuard<TorrentMaps>,
    rng: &mut SmallRng,
    requests: Drain<(AnnounceRequest, SocketAddr)>,
    responses: &mut Vec<(ConnectedResponse, SocketAddr)>,
) {
    let peer_valid_until = ValidUntil::new(config.cleaning.max_peer_age);

    responses.extend(requests.map(|(request, src)| {
        let peer_ip = convert_ipv4_mapped_ipv6(src.ip());

        let response = match peer_ip {
            IpAddr::V4(ip) => handle_announce_request(
                config,
                rng,
                &mut torrents.ipv4,
                request,
                ip,
                peer_valid_until,
            ),
            IpAddr::V6(ip) => handle_announce_request(
                config,
                rng,
                &mut torrents.ipv6,
                request,
                ip,
                peer_valid_until,
            ),
        };

        (ConnectedResponse::Announce(response), src)
    }));
}
