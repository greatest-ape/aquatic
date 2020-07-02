use std::net::{SocketAddr};
use std::io::ErrorKind;
use std::io::{Read, Write};

use either::Either;
use hashbrown::HashMap;
use log::info;
use mio::Token;
use mio::net::TcpStream;
use native_tls::{TlsAcceptor, TlsStream, MidHandshakeTlsStream};

use aquatic_common_tcp::network::stream::Stream;

use crate::common::*;
use crate::protocol::Request;


#[derive(Debug)]
pub enum RequestReadError {
    NeedMoreData,
    Invalid,
    StreamEnded,
    Io(::std::io::Error),
    Parse(::httparse::Error),
}


pub struct EstablishedConnection {
    stream: Stream,
    pub peer_addr: SocketAddr,
    buf: [u8; 1024],
    bytes_read: usize,
}


impl EstablishedConnection {
    fn new(stream: Stream) -> Self {
        let peer_addr = stream.get_peer_addr();

        Self {
            stream,
            peer_addr,
            buf: [0; 1024], // FIXME: fixed size is stupid
            bytes_read: 0,
        }
    }

    pub fn read_request(&mut self) -> Result<Request, RequestReadError> {
        match self.stream.read(&mut self.buf[self.bytes_read..]){
            Ok(0) => {
                return Err(RequestReadError::StreamEnded);
            }
            Ok(bytes_read) => {
                self.bytes_read += bytes_read;

                info!("parse request read {} bytes", bytes_read);
            },
            Err(err) if err.kind() == ErrorKind::WouldBlock => {
                return Err(RequestReadError::NeedMoreData);
            },
            Err(err) => {
                return Err(RequestReadError::Io(err));
            }
        }

        let mut headers = [httparse::EMPTY_HEADER; 16];
        let mut request = httparse::Request::new(&mut headers);

        let request = match request.parse(&self.buf[..self.bytes_read]){
            Ok(httparse::Status::Complete(_)) => {
                let result = if let Some(request) = Request::from_http(request){
                    Ok(request)
                } else {
                    Err(RequestReadError::Invalid)
                };

                self.bytes_read = 0;

                result
            },
            Ok(httparse::Status::Partial) => {
                Err(RequestReadError::NeedMoreData)
            },
            Err(err) => {
                self.bytes_read = 0;

                Err(RequestReadError::Parse(err))
            }
        };

        request
    }

    pub fn send_response(&mut self, body: &[u8]) -> ::std::io::Result<()> {
        let mut response = Vec::new();

        response.extend_from_slice(b"HTTP/1.1 200 OK\r\n");
        response.extend_from_slice(b"Content-Length: ");
        response.extend_from_slice(format!("{}", body.len() + 2).as_bytes());
        response.extend_from_slice(b"\r\n\r\n");
        response.extend_from_slice(body);
        response.extend_from_slice(b"\r\n");

        self.stream.write(&response)?;
        self.stream.flush()?;

        Ok(())
    }
}


pub enum HandshakeMachine {
    TcpStream(TcpStream),
    TlsMidHandshake(MidHandshakeTlsStream<TcpStream>),
}


impl <'a>HandshakeMachine {
    #[inline]
    fn new(tcp_stream: TcpStream) -> Self {
        Self::TcpStream(tcp_stream)
    }

    #[inline]
    pub fn advance(
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
                    log::debug!("established connection");
                    
                    let established_connection = EstablishedConnection::new(
                        Stream::TcpStream(stream)
                    );

                    (Some(Either::Left(established_connection)), false)
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
    pub inner: Either<EstablishedConnection, HandshakeMachine>,
}


impl Connection {
    #[inline]
    pub fn new(
        use_tls: bool,
        valid_until: ValidUntil,
        tcp_stream: TcpStream,
    ) -> Self {
        let inner = if use_tls {
            Either::Right(HandshakeMachine::new(tcp_stream))
        } else {
            // If no TLS should be used, just go directly to established
            // connection
            Either::Left(EstablishedConnection::new(Stream::TcpStream(tcp_stream)))
        };

        Self {
            valid_until,
            inner,
        }
    }
}


pub type ConnectionMap = HashMap<Token, Connection>;