use std::time::Instant;

use anyhow::Context;
use mio::Poll;
use socket2::{Domain, Protocol, Socket, Type};

use crate::config::Config;

use super::ConnectionMap;

pub fn create_listener(config: &Config) -> ::anyhow::Result<::std::net::TcpListener> {
    let builder = if config.network.address.is_ipv4() {
        Socket::new(Domain::IPV4, Type::STREAM, Some(Protocol::TCP))
    } else {
        Socket::new(Domain::IPV6, Type::STREAM, Some(Protocol::TCP))
    }
    .context("Couldn't create socket2::Socket")?;

    if config.network.ipv6_only {
        builder
            .set_only_v6(true)
            .context("Couldn't put socket in ipv6 only mode")?
    }

    builder
        .set_nonblocking(true)
        .context("Couldn't put socket in non-blocking mode")?;
    builder
        .set_reuse_port(true)
        .context("Couldn't put socket in reuse_port mode")?;
    builder
        .bind(&config.network.address.into())
        .with_context(|| format!("Couldn't bind socket to address {}", config.network.address))?;
    builder
        .listen(128)
        .context("Couldn't listen for connections on socket")?;

    Ok(builder.into())
}

// Close and remove inactive connections
pub fn remove_inactive_connections(
    mut connections: ConnectionMap,
    poll: &mut Poll,
) -> ConnectionMap {
    let now = Instant::now();

    let mut retained_connections = ConnectionMap::default();

    for (token, connection) in connections.drain() {
        if connection.valid_until.0 < now {
            connection.deregister(poll).close();
        } else {
            retained_connections.insert(token, connection);
        }
    }

    retained_connections
}
