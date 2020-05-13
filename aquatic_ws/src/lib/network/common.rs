use std::net::{SocketAddr};
use std::io::{Read, Write};

use hashbrown::HashMap;
use mio::Token;
use mio::net::TcpStream;
use native_tls::{TlsStream, MidHandshakeTlsStream};
use tungstenite::WebSocket;
use tungstenite::handshake::{MidHandshake, server::ServerHandshake};

use crate::common::*;


#[derive(Clone, Copy, Debug)]
pub struct DebugCallback;

impl ::tungstenite::handshake::server::Callback for DebugCallback {
    fn on_request(
        self,
        request: &::tungstenite::handshake::server::Request,
        response: ::tungstenite::handshake::server::Response,
    ) -> Result<::tungstenite::handshake::server::Response, ::tungstenite::handshake::server::ErrorResponse> {
        println!("request: {:#?}", request);
        println!("response: {:#?}", response);

        Ok(response)
    }
}


pub enum Stream {
    TcpStream(TcpStream),
    TlsStream(TlsStream<TcpStream>),
}


impl Stream {
    pub fn get_peer_addr(&self) -> SocketAddr {
        match self {
            Self::TcpStream(stream) => stream.peer_addr().unwrap(),
            Self::TlsStream(stream) => stream.get_ref().peer_addr().unwrap(),
        }
    }
}


impl Read for Stream {
    fn read(&mut self, buf: &mut [u8]) -> Result<usize, ::std::io::Error> {
        match self {
            Self::TcpStream(stream) => stream.read(buf),
            Self::TlsStream(stream) => stream.read(buf),
        }
    }
}


impl Write for Stream {
    fn write(&mut self, buf: &[u8]) -> ::std::io::Result<usize> {
        match self {
            Self::TcpStream(stream) => stream.write(buf),
            Self::TlsStream(stream) => stream.write(buf),
        }
    }

    fn flush(&mut self) -> ::std::io::Result<()> {
        match self {
            Self::TcpStream(stream) => stream.flush(),
            Self::TlsStream(stream) => stream.flush(),
        }
    }
}


pub struct EstablishedWs<S> {
    pub ws: WebSocket<S>,
    pub peer_addr: SocketAddr,
}


pub enum ConnectionStage {
    TcpStream(TcpStream),
    TlsStream(TlsStream<TcpStream>),
    TlsMidHandshake(MidHandshakeTlsStream<TcpStream>),
    WsMidHandshake(MidHandshake<ServerHandshake<Stream, DebugCallback>>),
    EstablishedWs(EstablishedWs<Stream>),
}


impl ConnectionStage {
    pub fn is_established(&self) -> bool {
        match self {
            Self::EstablishedWs(_) => true,
            _ => false,
        }
    }
}


pub struct Connection {
    pub valid_until: ValidUntil,
    pub stage: ConnectionStage,
}


pub type ConnectionMap = HashMap<Token, Connection>;