use std::time::Instant;

use anyhow::Context;
use mio::Token;
use socket2::{Socket, Domain, Type, Protocol};

use crate::config::Config;

use super::connection::*;


pub fn create_listener(
    config: &Config
) -> ::anyhow::Result<::std::net::TcpListener> {
    let builder = if config.network.address.is_ipv4(){
        Socket::new(Domain::ipv4(), Type::stream(), Some(Protocol::tcp()))
    } else {
        Socket::new(Domain::ipv6(), Type::stream(), Some(Protocol::tcp()))
    }.context("Couldn't create socket2::Socket")?;

    if config.network.ipv6_only {
        builder.set_only_v6(true)
            .context("Couldn't put socket in ipv6 only mode")?
    }

    builder.set_nonblocking(true)
        .context("Couldn't put socket in non-blocking mode")?;
    builder.set_reuse_port(true)
        .context("Couldn't put socket in reuse_port mode")?;
    builder.bind(&config.network.address.into()).with_context(||
        format!("Couldn't bind socket to address {}", config.network.address)
    )?;
    builder.listen(128)
        .context("Couldn't listen for connections on socket")?;

    Ok(builder.into_tcp_listener())
}


/// Don't bother with deregistering from Poll. In my understanding, this is
/// done automatically when the stream is dropped, as long as there are no
/// other references to the file descriptor, such as when it is accessed
/// in multiple threads.
pub fn remove_connection_if_exists(
    connections: &mut ConnectionMap,
    token: Token,
){
    if let Some(mut connection) = connections.remove(&token){
        connection.close();
    }
}


// Close and remove inactive connections
pub fn remove_inactive_connections(
    connections: &mut ConnectionMap,
){
    let now = Instant::now();

    connections.retain(|_, connection| {
        if connection.valid_until.0 < now {
            connection.close();

            false
        } else {
            true
        }
    });

    connections.shrink_to_fit();
}