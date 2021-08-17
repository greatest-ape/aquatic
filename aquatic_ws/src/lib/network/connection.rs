use std::io::{Read, Write};
use std::net::SocketAddr;

use either::Either;
use hashbrown::HashMap;
use log::info;
use mio::net::TcpStream;
use mio::{Poll, Token};
use native_tls::{MidHandshakeTlsStream, TlsAcceptor, TlsStream};
use tungstenite::handshake::{server::NoCallback, HandshakeError, MidHandshake};
use tungstenite::protocol::WebSocketConfig;
use tungstenite::ServerHandshake;
use tungstenite::WebSocket;

use crate::common::*;

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

    #[inline]
    pub fn deregister(&mut self, poll: &mut Poll) -> ::std::io::Result<()> {
        match self {
            Self::TcpStream(stream) => poll.registry().deregister(stream),
            Self::TlsStream(stream) => poll.registry().deregister(stream.get_mut()),
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
        bufs: &mut [::std::io::IoSliceMut<'_>],
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
    fn write_vectored(&mut self, bufs: &[::std::io::IoSlice<'_>]) -> ::std::io::Result<usize> {
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

enum HandshakeMachine {
    TcpStream(TcpStream),
    TlsStream(TlsStream<TcpStream>),
    TlsMidHandshake(MidHandshakeTlsStream<TcpStream>),
    WsMidHandshake(MidHandshake<ServerHandshake<Stream, NoCallback>>),
}

impl HandshakeMachine {
    #[inline]
    fn new(tcp_stream: TcpStream) -> Self {
        Self::TcpStream(tcp_stream)
    }

    #[inline]
    fn advance(
        self,
        ws_config: WebSocketConfig,
        opt_tls_acceptor: &Option<TlsAcceptor>, // If set, run TLS
    ) -> (Option<Either<EstablishedWs, Self>>, bool) {
        // bool = stop looping
        match self {
            HandshakeMachine::TcpStream(stream) => {
                if let Some(tls_acceptor) = opt_tls_acceptor {
                    Self::handle_tls_handshake_result(tls_acceptor.accept(stream))
                } else {
                    let handshake_result = ::tungstenite::accept_with_config(
                        Stream::TcpStream(stream),
                        Some(ws_config),
                    );

                    Self::handle_ws_handshake_result(handshake_result)
                }
            }
            HandshakeMachine::TlsStream(stream) => {
                let handshake_result = ::tungstenite::accept(Stream::TlsStream(stream));

                Self::handle_ws_handshake_result(handshake_result)
            }
            HandshakeMachine::TlsMidHandshake(handshake) => {
                Self::handle_tls_handshake_result(handshake.handshake())
            }
            HandshakeMachine::WsMidHandshake(handshake) => {
                Self::handle_ws_handshake_result(handshake.handshake())
            }
        }
    }

    #[inline]
    fn handle_tls_handshake_result(
        result: Result<TlsStream<TcpStream>, ::native_tls::HandshakeError<TcpStream>>,
    ) -> (Option<Either<EstablishedWs, Self>>, bool) {
        match result {
            Ok(stream) => {
                ::log::trace!(
                    "established tls handshake with peer with addr: {:?}",
                    stream.get_ref().peer_addr()
                );

                (Some(Either::Right(Self::TlsStream(stream))), false)
            },
            Err(native_tls::HandshakeError::WouldBlock(handshake)) => {
                (Some(Either::Right(Self::TlsMidHandshake(handshake))), true)
            }
            Err(native_tls::HandshakeError::Failure(err)) => {
                info!("tls handshake error: {}", err);

                (None, false)
            }
        }
    }

    #[inline]
    fn handle_ws_handshake_result(
        result: Result<WebSocket<Stream>, HandshakeError<ServerHandshake<Stream, NoCallback>>>,
    ) -> (Option<Either<EstablishedWs, Self>>, bool) {
        match result {
            Ok(mut ws) => {
                let peer_addr = ws.get_mut().get_peer_addr();

                ::log::trace!(
                    "established ws handshake with peer with addr: {:?}",
                    peer_addr
                );

                let established_ws = EstablishedWs {
                    ws,
                    peer_addr,
                };

                (Some(Either::Left(established_ws)), false)
            }
            Err(HandshakeError::Interrupted(handshake)) => (
                Some(Either::Right(HandshakeMachine::WsMidHandshake(handshake))),
                true,
            ),
            Err(HandshakeError::Failure(err)) => {
                info!("ws handshake error: {}", err);

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
    ws_config: WebSocketConfig,
    pub valid_until: ValidUntil,
    inner: Either<EstablishedWs, HandshakeMachine>,
}

/// Create from TcpStream. Run `advance_handshakes` until `get_established_ws`
/// returns Some(EstablishedWs).
///
/// advance_handshakes takes ownership of self because the TLS and WebSocket
/// handshake methods do. get_established_ws doesn't, since work can be done
/// on a mutable reference to a tungstenite websocket, and this way, the whole
/// Connection doesn't have to be removed from and reinserted into the
/// TorrentMap. This is also the reason for wrapping Container.inner in an
/// Either instead of combining all states into one structure just having a
/// single method for advancing handshakes and maybe returning a websocket.
impl Connection {
    #[inline]
    pub fn new(ws_config: WebSocketConfig, valid_until: ValidUntil, tcp_stream: TcpStream) -> Self {
        Self {
            ws_config,
            valid_until,
            inner: Either::Right(HandshakeMachine::new(tcp_stream)),
        }
    }

    #[inline]
    pub fn get_established_ws(&mut self) -> Option<&mut EstablishedWs> {
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
                let ws_config = self.ws_config;

                let (opt_inner, stop_loop) = machine.advance(ws_config, opt_tls_acceptor);

                let opt_new_self = opt_inner.map(|inner| Self {
                    ws_config,
                    valid_until,
                    inner,
                });

                (opt_new_self, stop_loop)
            }
        }
    }

    #[inline]
    pub fn close(&mut self) {
        if let Either::Left(ref mut ews) = self.inner {
            if ews.ws.can_read() {
                if let Err(err) = ews.ws.close(None) {
                    ::log::info!("error closing ws: {}", err);
                }

                // Required after ws.close()
                if let Err(err) = ews.ws.write_pending() {
                    ::log::info!("error writing pending messages after closing ws: {}", err)
                }
            }
        }
    }

    pub fn deregister(&mut self, poll: &mut Poll) -> ::std::io::Result<()> {
        use Either::{Left, Right};

        match self.inner {
            Left(EstablishedWs { ref mut ws, .. }) => ws.get_mut().deregister(poll),
            Right(HandshakeMachine::TcpStream(ref mut stream)) => {
                poll.registry().deregister(stream)
            }
            Right(HandshakeMachine::TlsMidHandshake(ref mut handshake)) => {
                poll.registry().deregister(handshake.get_mut())
            }
            Right(HandshakeMachine::TlsStream(ref mut stream)) => {
                poll.registry().deregister(stream.get_mut())
            }
            Right(HandshakeMachine::WsMidHandshake(ref mut handshake)) => {
                handshake.get_mut().get_mut().deregister(poll)
            }
        }
    }
}

pub type ConnectionMap = HashMap<Token, Connection>;
