use std::fs::File;
use std::io::Read;
use std::time::Instant;

use mio::{Poll, Token};
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
    let mut file = File::open(&config.network.pkcs12_path)
        .expect("open pkcs12 file");

    file.read_to_end(&mut identity_bytes).expect("read pkcs12 file");

    let identity = Identity::from_pkcs12(
        &mut identity_bytes,
        &config.network.pkcs12_password
    ).expect("create pkcs12 identity");

    let acceptor = TlsAcceptor::new(identity)   
        .expect("create TlsAcceptor");

    acceptor
}


/// FIXME
pub fn close_and_deregister_connection(
    poll: &mut Poll,
    connection: &mut Connection,
){
    match connection.stage {
        ConnectionStage::Stream(ref mut stream) => {
            /*
            poll.registry()
                .deregister(stream)
                .unwrap();
            */
        },
        ConnectionStage::TlsMidHandshake(ref mut handshake) => {
            /*
            poll.registry()
                .deregister(handshake.get_mut())
                .unwrap();
            */
        },
        ConnectionStage::WsHandshake(ref mut handshake) => {
            /*
            poll.registry()
                .deregister(handshake.get_mut().get_mut())
                .unwrap();
                */
        },
        ConnectionStage::EstablishedWs(ref mut established_ws) => {
            if established_ws.ws.can_read(){
                established_ws.ws.close(None).unwrap();

                // Needs to be done after ws.close()
                if let Err(err) = established_ws.ws.write_pending(){
                    dbg!(err);
                }
            }

            /*
            poll.registry()
                .deregister(established_ws.ws.get_mut())
                .unwrap();
                */
        },
    }
}


pub fn remove_connection_if_exists(
    poll: &mut Poll,
    connections: &mut ConnectionMap,
    token: Token,
){
    if let Some(mut connection) = connections.remove(&token){
        close_and_deregister_connection(poll, &mut connection);

        connections.remove(&token);
    }
}

// Close and remove inactive connections
pub fn remove_inactive_connections(
    poll: &mut Poll,
    connections: &mut ConnectionMap,
){
    let now = Instant::now();

    connections.retain(|_, connection| {
        if connection.valid_until.0 < now {
            close_and_deregister_connection(poll, connection);

            println!("closing connection, it is inactive");

            false
        } else {
            println!("keeping connection, it is still active");

            true
        }
    });

    connections.shrink_to_fit();
}