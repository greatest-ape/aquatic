use std::net::SocketAddr;

use anyhow::Context;
use socket2::{Socket, Domain, Type, Protocol};

pub fn create_listener(
    address: SocketAddr,
    ipv6_only: bool
) -> ::anyhow::Result<::std::net::TcpListener> {
    let builder = if address.is_ipv4(){
        Socket::new(Domain::ipv4(), Type::stream(), Some(Protocol::tcp()))
    } else {
        Socket::new(Domain::ipv6(), Type::stream(), Some(Protocol::tcp()))
    }.context("Couldn't create socket2::Socket")?;

    if ipv6_only {
        builder.set_only_v6(true)
            .context("Couldn't put socket in ipv6 only mode")?
    }

    builder.set_nonblocking(true)
        .context("Couldn't put socket in non-blocking mode")?;
    builder.set_reuse_port(true)
        .context("Couldn't put socket in reuse_port mode")?;
    builder.bind(&address.into()).with_context(||
        format!("Couldn't bind socket to address {}", address)
    )?;
    builder.listen(128)
        .context("Couldn't listen for connections on socket")?;

    Ok(builder.into_tcp_listener())
}