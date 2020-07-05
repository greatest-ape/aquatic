
use std::fs::File;
use std::io::Read;
use std::net::SocketAddr;

use anyhow::Context;
use native_tls::{Identity, TlsAcceptor};
use socket2::{Socket, Domain, Type, Protocol};

use crate::config::TlsConfig;


pub fn create_tls_acceptor(
    config: &TlsConfig,
) -> anyhow::Result<Option<TlsAcceptor>> {
    if config.use_tls {
        let mut identity_bytes = Vec::new();
        let mut file = File::open(&config.tls_pkcs12_path)
            .context("Couldn't open pkcs12 identity file")?;

        file.read_to_end(&mut identity_bytes)
            .context("Couldn't read pkcs12 identity file")?;

        let identity = Identity::from_pkcs12(
            &identity_bytes[..],
            &config.tls_pkcs12_password
        ).context("Couldn't parse pkcs12 identity file")?;

        let acceptor = TlsAcceptor::new(identity)
            .context("Couldn't create TlsAcceptor from pkcs12 identity")?;

        Ok(Some(acceptor))
    } else {
        Ok(None)
    }
}


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