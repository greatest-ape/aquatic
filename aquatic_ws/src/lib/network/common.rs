use std::net::{SocketAddr};
use std::io::{Read, Write};

use either::Either;
use hashbrown::HashMap;
use mio::Token;
use mio::net::TcpStream;
use native_tls::{TlsAcceptor, TlsStream, MidHandshakeTlsStream};
use tungstenite::WebSocket;
use tungstenite::handshake::{MidHandshake, HandshakeError};
use tungstenite::server::ServerHandshake;

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
    #[inline]
    pub fn get_peer_addr(&self) -> SocketAddr {
        match self {
            Self::TcpStream(stream) => stream.peer_addr().unwrap(),
            Self::TlsStream(stream) => stream.get_ref().peer_addr().unwrap(),
        }
    }
}


impl Read for Stream {
    #[inline]
    fn read(&mut self, buf: &mut [u8]) -> Result<usize, ::std::io::Error> {
        match self {
            Self::TcpStream(stream) => stream.read(buf),
            Self::TlsStream(stream) => stream.read(buf),
        }
    }

    /// Not used but provided for completeness
    #[inline]
    fn read_vectored(
        &mut self,
        bufs: &mut [::std::io::IoSliceMut<'_>]
    ) -> ::std::io::Result<usize> {
        match self {
            Self::TcpStream(stream) => stream.read_vectored(bufs),
            Self::TlsStream(stream) => stream.read_vectored(bufs),
        }
    }
}


impl Write for Stream {
    #[inline]
    fn write(&mut self, buf: &[u8]) -> ::std::io::Result<usize> {
        match self {
            Self::TcpStream(stream) => stream.write(buf),
            Self::TlsStream(stream) => stream.write(buf),
        }
    }

    /// Not used but provided for completeness
    #[inline]
    fn write_vectored(
        &mut self,
        bufs: &[::std::io::IoSlice<'_>]
    ) -> ::std::io::Result<usize> {
        match self {
            Self::TcpStream(stream) => stream.write_vectored(bufs),
            Self::TlsStream(stream) => stream.write_vectored(bufs),
        }
    }

    #[inline]
    fn flush(&mut self) -> ::std::io::Result<()> {
        match self {
            Self::TcpStream(stream) => stream.flush(),
            Self::TlsStream(stream) => stream.flush(),
        }
    }
}


pub enum HandshakeMachine {
    TcpStream(TcpStream),
    TlsStream(TlsStream<TcpStream>),
    TlsMidHandshake(MidHandshakeTlsStream<TcpStream>),
    WsMidHandshake(MidHandshake<ServerHandshake<Stream, DebugCallback>>),
}


impl HandshakeMachine {
    #[inline]
    pub fn new(tcp_stream: TcpStream) -> Self {
        Self::TcpStream(tcp_stream)
    }

    #[inline]
    pub fn advance(
        self,
        opt_tls_acceptor: &Option<TlsAcceptor>, // If set, run TLS
    ) -> (Option<Either<EstablishedWs, Self>>, bool) { // bool = stop looping
        match self {
            HandshakeMachine::TcpStream(stream) => {
                if let Some(tls_acceptor) = opt_tls_acceptor {
                    Self::handle_tls_handshake_result(
                        tls_acceptor.accept(stream)
                    )
                } else {
                    let handshake_result = ::tungstenite::server::accept_hdr(
                        Stream::TcpStream(stream),
                        DebugCallback
                    );

                    Self::handle_ws_handshake_result(handshake_result)
                }
            },
            HandshakeMachine::TlsStream(stream) => {
                let handshake_result = ::tungstenite::server::accept_hdr(
                    Stream::TlsStream(stream),
                    DebugCallback
                );

                Self::handle_ws_handshake_result(handshake_result)
            },
            HandshakeMachine::TlsMidHandshake(handshake) => {
                Self::handle_tls_handshake_result(handshake.handshake())
            },
            HandshakeMachine::WsMidHandshake(handshake) => {
                Self::handle_ws_handshake_result(handshake.handshake())
            },
        }
    }

    #[inline]
    fn handle_tls_handshake_result(
        result: Result<TlsStream<TcpStream>, ::native_tls::HandshakeError<TcpStream>>,
    ) -> (Option<Either<EstablishedWs, Self>>, bool) {
        match result {
            Ok(stream) => {
                println!("handshake established");

                (Some(Either::Right(Self::TlsStream(stream))), false)
            },
            Err(native_tls::HandshakeError::WouldBlock(handshake)) => {
                println!("interrupted");

                (Some(Either::Right(Self::TlsMidHandshake(handshake))), true)
            },
            Err(native_tls::HandshakeError::Failure(err)) => {
                dbg!(err);

                (None, false)
            }
        }
    }

    #[inline]
    fn handle_ws_handshake_result(
        result: Result<WebSocket<Stream>, HandshakeError<ServerHandshake<Stream, DebugCallback>>> ,
    ) -> (Option<Either<EstablishedWs, Self>>, bool) {
        match result {
            Ok(mut ws) => {
                println!("handshake established");

                let peer_addr = ws.get_mut().get_peer_addr();

                let established_ws = EstablishedWs {
                    ws,
                    peer_addr,
                };

                (Some(Either::Left(established_ws)), false)
            },
            Err(HandshakeError::Interrupted(handshake)) => {
                println!("interrupted");

                (Some(Either::Right(HandshakeMachine::WsMidHandshake(handshake))), true)
            },
            Err(HandshakeError::Failure(err)) => {
                dbg!(err);

                (None, false)
            }
        }
    }
}


pub struct EstablishedWs {
    pub ws: WebSocket<Stream>,
    pub peer_addr: SocketAddr,
}


pub struct Connection {
    pub valid_until: ValidUntil,
    inner: Either<EstablishedWs, HandshakeMachine>,
}


/// Create from TcpStream. Run `advance_handshakes` until `get_established_ws`
/// returns Some(EstablishedWs).
impl Connection {
    #[inline]
    pub fn new(
        valid_until: ValidUntil,
        tcp_stream: TcpStream,
    ) -> Self {
        Self {
            valid_until,
            inner: Either::Right(HandshakeMachine::TcpStream(tcp_stream))
        }
    }

    #[inline]
    pub fn get_established_ws<'a>(&mut self) -> Option<&mut EstablishedWs> {
        match self.inner {
            Either::Left(ref mut ews) => Some(ews),
            Either::Right(_) => None,
        }
    }

    #[inline]
    pub fn advance_handshakes(
        self,
        opt_tls_acceptor: &Option<TlsAcceptor>,
        valid_until: ValidUntil,
    ) -> (Option<Self>, bool) {
        match self.inner {
            Either::Left(_) => (Some(self), false),
            Either::Right(machine) => {
                let (opt_inner, stop_loop) = machine.advance(opt_tls_acceptor);

                let opt_new_self = opt_inner.map(|inner| Self {
                    valid_until,
                    inner
                });

                (opt_new_self, stop_loop)
            }
        }
    }

    #[inline]
    pub fn close(&mut self){
        if let Either::Left(ref mut ews) = self.inner {
            if ews.ws.can_read(){
                ews.ws.close(None).unwrap();

                // Needs to be done after ws.close()
                if let Err(err) = ews.ws.write_pending(){
                    dbg!(err);
                }
            }
        }
    }
}


pub type ConnectionMap = HashMap<Token, Connection>;