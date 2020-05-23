use std::time::Instant;

use anyhow::Context;
use mio::Token;
use net2::{TcpBuilder, unix::UnixTcpBuilderExt};

use crate::config::Config;

use super::connection::*;


pub fn create_listener(
    config: &Config
) -> ::anyhow::Result<::std::net::TcpListener> {
    let mut builder = &if config.network.address.is_ipv4(){
        TcpBuilder::new_v4()
    } else {
        TcpBuilder::new_v6()
    }?;

    if config.network.ipv6_only {
        builder = builder.only_v6(true)
            .context("Failed setting ipv6_only to true")?
    }

    builder = builder.reuse_port(true)?;
    builder = builder.bind(&config.network.address)?;

    let listener = builder.listen(128)?;

    listener.set_nonblocking(true)?;

    Ok(listener)
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