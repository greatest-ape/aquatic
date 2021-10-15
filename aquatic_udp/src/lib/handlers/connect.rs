use std::net::SocketAddr;
use std::vec::Drain;

use parking_lot::MutexGuard;
use rand::{rngs::StdRng, Rng};

use aquatic_udp_protocol::*;

use crate::common::*;
use crate::config::Config;

#[inline]
pub fn handle_connect_requests(
    config: &Config,
    connections: &mut MutexGuard<ConnectionMap>,
    rng: &mut StdRng,
    requests: Drain<(ConnectRequest, SocketAddr)>,
    responses: &mut Vec<(Response, SocketAddr)>,
) {
    let valid_until = ValidUntil::new(config.cleaning.max_connection_age);

    responses.extend(requests.map(|(request, src)| {
        let connection_id = ConnectionId(rng.gen());

        let key = ConnectionKey {
            connection_id,
            socket_addr: src,
        };

        connections.insert(key, valid_until);

        let response = Response::Connect(ConnectResponse {
            connection_id,
            transaction_id: request.transaction_id,
        });

        (response, src)
    }));
}
