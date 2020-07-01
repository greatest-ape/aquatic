use std::net::{SocketAddr};
use std::io::{Read, Write};
use std::io::ErrorKind;

use either::Either;
use hashbrown::HashMap;
use log::info;
use mio::Token;
use mio::net::TcpStream;
use native_tls::{TlsAcceptor, TlsStream, MidHandshakeTlsStream};

use crate::common::*;
use crate::protocol::{Request, Response};


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


#[derive(Debug)]
pub enum RequestParseError {
    NeedMoreData,
    Invalid,
    Incomplete,
    Io(::std::io::Error),
    Parse(::httparse::Error)
}


pub struct EstablishedConnection {
    stream: Stream,
    pub peer_addr: SocketAddr,
    buf: Vec<u8>,
}


impl EstablishedConnection {
    fn new(stream: Stream) -> Self {
        let peer_addr = stream.get_peer_addr();

        Self {
            stream,
            peer_addr,
            buf: Vec::new(), // FIXME: with capacity of like 100?
        }
    }

    pub fn parse_request(&mut self) -> Result<Request, RequestParseError> {
        match self.stream.read(&mut self.buf){
            Ok(0) => {
                // FIXME: finished reading completely here?
            },
            Ok(_) => {
                return Err(RequestParseError::NeedMoreData);
            },
            Err(err) if err.kind() == ErrorKind::WouldBlock => {
                return Err(RequestParseError::NeedMoreData);
            },
            Err(err) => {
                return Err(RequestParseError::Io(err));
            }
        }

        let mut headers = [httparse::EMPTY_HEADER; 1];
        let mut request = httparse::Request::new(&mut headers);

        let request = match request.parse(&self.buf){
            Ok(httparse::Status::Complete(_)) => {
                if let Some(request) = Request::from_http(request){
                    Ok(request)
                } else {
                    Err(RequestParseError::Invalid)
                }
            },
            Ok(httparse::Status::Partial) => {
                Err(RequestParseError::Incomplete)
            },
            Err(err) => {
                Err(RequestParseError::Parse(err))
            }
        };

        self.buf.clear();
        self.buf.shrink_to_fit();

        request
    }

    pub fn send_response(&mut self, body: &str) -> Result<(), RequestParseError> {
        let mut response = String::new();

        response.push_str("200 OK\r\n\r\n");
        response.push_str(body);

        match self.stream.write(response.as_bytes()){
            Ok(_) => Ok(()),
            Err(err) => {
                info!("send response: {:?}", err);

                Err(RequestParseError::Io(err))
            }
        }
    }
}


enum HandshakeMachine {
    TcpStream(TcpStream),
    TlsMidHandshake(MidHandshakeTlsStream<TcpStream>),
}


impl <'a>HandshakeMachine {
    #[inline]
    fn new(tcp_stream: TcpStream) -> Self {
        Self::TcpStream(tcp_stream)
    }

    #[inline]
    fn advance(
        self,
        opt_tls_acceptor: &Option<TlsAcceptor>, // If set, run TLS
    ) -> (Option<Either<EstablishedConnection, Self>>, bool) { // bool = stop looping
        match self {
            HandshakeMachine::TcpStream(stream) => {
                if let Some(tls_acceptor) = opt_tls_acceptor {
                    Self::handle_tls_handshake_result(
                        tls_acceptor.accept(stream)
                    )
                } else {
                    (Some(Either::Left(EstablishedConnection::new(Stream::TcpStream(stream)))), false)
                }
            },
            HandshakeMachine::TlsMidHandshake(handshake) => {
                Self::handle_tls_handshake_result(handshake.handshake())
            },
        }
    }

    #[inline]
    fn handle_tls_handshake_result(
        result: Result<TlsStream<TcpStream>, ::native_tls::HandshakeError<TcpStream>>,
    ) -> (Option<Either<EstablishedConnection, Self>>, bool) {
        match result {
            Ok(stream) => {
                (Some(Either::Left(EstablishedConnection::new(Stream::TlsStream(stream)))), false)
            },
            Err(native_tls::HandshakeError::WouldBlock(handshake)) => {
                (Some(Either::Right(Self::TlsMidHandshake(handshake))), true)
            },
            Err(native_tls::HandshakeError::Failure(err)) => {
                info!("tls handshake error: {}", err);

                (None, false)
            }
        }
    }
}


pub struct Connection {
    pub valid_until: ValidUntil,
    inner: Either<EstablishedConnection, HandshakeMachine>,
}


/// Create from TcpStream. Run `advance_handshakes` until `get_established_ws`
/// returns Some(EstablishedWs).
///
/// advance_handshakes takes ownership of self because the TLS handshake
/// methods does. get_established doesn't, since work can be done on a mutable
/// reference to a tls stream, and this way, the whole connection doesn't have
/// to be removed/inserted into the ConnectionMap
impl Connection {
    #[inline]
    pub fn new(
        valid_until: ValidUntil,
        tcp_stream: TcpStream,
    ) -> Self {
        Self {
            valid_until,
            inner: Either::Right(HandshakeMachine::new(tcp_stream))
        }
    }

    #[inline]
    pub fn get_established(&mut self) -> Option<&mut EstablishedConnection> {
        match self.inner {
            Either::Left(ref mut established) => Some(established),
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
                let (opt_inner, stop_loop) = machine.advance(
                    opt_tls_acceptor
                );

                let opt_new_self = opt_inner.map(|inner| Self {
                    valid_until,
                    inner
                });

                (opt_new_self, stop_loop)
            }
        }
    }
}


pub type ConnectionMap<'a> = HashMap<Token, Connection>;