use std::net::{SocketAddr};

use hashbrown::HashMap;
use mio::Token;
use mio::net::TcpStream;
use native_tls::TlsStream;
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


pub type Stream = TlsStream<TcpStream>;


pub struct EstablishedWs<S> {
    pub ws: WebSocket<S>,
    pub peer_addr: SocketAddr,
}


pub enum ConnectionStage {
    TcpStream(TcpStream),
    TlsMidHandshake(native_tls::MidHandshakeTlsStream<TcpStream>),
    TlsStream(Stream),
    WsHandshakeNoTls(MidHandshake<ServerHandshake<TcpStream, DebugCallback>>),
    WsHandshakeTls(MidHandshake<ServerHandshake<Stream, DebugCallback>>),
    EstablishedWsNoTls(EstablishedWs<TcpStream>),
    EstablishedWsTls(EstablishedWs<Stream>),
}


impl ConnectionStage {
    pub fn is_established(&self) -> bool {
        match self {
            Self::EstablishedWsTls(_) | Self::EstablishedWsNoTls(_) => true,
            _ => false,
        }
    }
}


pub struct Connection {
    pub valid_until: ValidUntil,
    pub stage: ConnectionStage,
}


pub type ConnectionMap = HashMap<Token, Connection>;