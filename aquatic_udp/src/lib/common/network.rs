use std::{net::SocketAddr, time::Instant};

use aquatic_common::AHashIndexMap;
pub use aquatic_common::{access_list::AccessList, ValidUntil};
pub use aquatic_udp_protocol::*;

#[derive(Default)]
pub struct ConnectionMap(AHashIndexMap<(ConnectionId, SocketAddr), ValidUntil>);

impl ConnectionMap {
    pub fn insert(
        &mut self,
        connection_id: ConnectionId,
        socket_addr: SocketAddr,
        valid_until: ValidUntil,
    ) {
        self.0.insert((connection_id, socket_addr), valid_until);
    }

    pub fn contains(&self, connection_id: ConnectionId, socket_addr: SocketAddr) -> bool {
        self.0.contains_key(&(connection_id, socket_addr))
    }

    pub fn clean(&mut self) {
        let now = Instant::now();

        self.0.retain(|_, v| v.0 > now);
        self.0.shrink_to_fit();
    }
}
