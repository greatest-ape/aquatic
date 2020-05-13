use std::fs::File;
use std::io::Read;
use std::time::Instant;

use either::Either;
use mio::Token;
use native_tls::{Identity, TlsAcceptor};
use net2::{TcpBuilder, unix::UnixTcpBuilderExt};

use crate::config::Config;

use super::common::*;


pub fn create_listener(config: &Config) -> ::std::net::TcpListener {
    let mut builder = &{
        if config.network.address.is_ipv4(){
            TcpBuilder::new_v4().expect("socket: build")
        } else {
            TcpBuilder::new_v6().expect("socket: build")
        }
    };

    builder = builder.reuse_port(true)
        .expect("socket: set reuse port");

    builder = builder.bind(&config.network.address)
        .expect(&format!("socket: bind to {}", &config.network.address));

    let listener = builder.listen(128)
        .expect("tcpbuilder to tcp listener");

    listener.set_nonblocking(true)
        .expect("socket: set nonblocking");

    listener
}


pub fn create_tls_acceptor(
    config: &Config,
) -> TlsAcceptor {
    let mut identity_bytes = Vec::new();
    let mut file = File::open(&config.network.tls_pkcs12_path)
        .expect("open pkcs12 file");

    file.read_to_end(&mut identity_bytes).expect("read pkcs12 file");

    let identity = Identity::from_pkcs12(
        &mut identity_bytes,
        &config.network.tls_pkcs12_password
    ).expect("create pkcs12 identity");

    let acceptor = TlsAcceptor::new(identity)   
        .expect("create TlsAcceptor");

    acceptor
}


pub fn close_connection(connection: &mut Connection){
    if let Either::Left(ref mut ews) = connection.inner {
        if ews.ws.can_read(){
            ews.ws.close(None).unwrap();

            // Needs to be done after ws.close()
            if let Err(err) = ews.ws.write_pending(){
                dbg!(err);
            }
        }
    }
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
        close_connection(&mut connection);
    }
}


// Close and remove inactive connections
pub fn remove_inactive_connections(
    connections: &mut ConnectionMap,
){
    let now = Instant::now();

    connections.retain(|_, connection| {
        if connection.valid_until.0 < now {
            close_connection(connection);

            println!("closing connection, it is inactive");

            false
        } else {
            println!("keeping connection, it is still active");

            true
        }
    });

    connections.shrink_to_fit();
}