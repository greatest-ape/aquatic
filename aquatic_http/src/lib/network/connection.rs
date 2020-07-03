use std::net::{SocketAddr};
use std::io::ErrorKind;
use std::io::{Read, Write};
use std::sync::Arc;

use either::Either;
use hashbrown::HashMap;
use log::info;
use mio::Token;
use mio::net::TcpStream;
use native_tls::{TlsAcceptor, MidHandshakeTlsStream};

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
    buf: Vec<u8>,
    bytes_read: usize,
}


impl EstablishedConnection {
    fn new(stream: Stream) -> Self {
        let peer_addr = stream.get_peer_addr();

        Self {
            stream,
            peer_addr,
            buf: Vec::new(),
            bytes_read: 0,
        }
    }

    pub fn read_request(&mut self) -> Result<Request, RequestReadError> {
        if self.buf.len() - self.bytes_read < 512 {
            self.buf.extend_from_slice(&[0; 1024]);
        }

        match self.stream.read(&mut self.buf[self.bytes_read..]){
            Ok(0) => {
                self.clear_buffer();

                return Err(RequestReadError::StreamEnded);
            }
            Ok(bytes_read) => {
                self.bytes_read += bytes_read;

                info!("read_request read {} bytes", bytes_read);
            },
            Err(err) if err.kind() == ErrorKind::WouldBlock => {
                return Err(RequestReadError::NeedMoreData);
            },
            Err(err) => {
                self.clear_buffer();

                return Err(RequestReadError::Io(err));
            }
        }

        let mut headers = [httparse::EMPTY_HEADER; 16];
        let mut http_request = httparse::Request::new(&mut headers);

        match http_request.parse(&self.buf[..self.bytes_read]){
            Ok(httparse::Status::Complete(_)) => {
                let opt_request = http_request.path.and_then(
                    Request::from_http_get_path
                );

                self.clear_buffer();

                if let Some(request) = opt_request {
                    Ok(request)
                } else {
                    Err(RequestReadError::Invalid)
                }
            },
            Ok(httparse::Status::Partial) => {
                Err(RequestReadError::NeedMoreData)
            },
            Err(err) => {
                self.clear_buffer();

                Err(RequestReadError::Parse(err))
            }
        }
    }

    pub fn send_response(&mut self, body: &[u8]) -> ::std::io::Result<()> {
        let mut response = Vec::new();

        response.extend_from_slice(b"HTTP/1.1 200 OK\r\nContent-Length: ");
        response.extend_from_slice(format!("{}", body.len() + 2).as_bytes());
        response.extend_from_slice(b"\r\n\r\n");
        response.extend_from_slice(body);
        response.extend_from_slice(b"\r\n");

        self.stream.write(&response)?;
        self.stream.flush()?;

        Ok(())
    }

    pub fn clear_buffer(&mut self){
        self.bytes_read = 0;
        self.buf = Vec::new();
    }
}


enum HandshakeMachineInner {
    TcpStream(TcpStream),
    TlsMidHandshake(MidHandshakeTlsStream<TcpStream>),
}


pub struct TlsHandshakeMachine {
    tls_acceptor: Arc<TlsAcceptor>,
    inner: HandshakeMachineInner,
}


impl <'a>TlsHandshakeMachine {
    #[inline]
    fn new(
        tls_acceptor: Arc<TlsAcceptor>,
        tcp_stream: TcpStream
    ) -> Self {
        Self {
            tls_acceptor,
            inner: HandshakeMachineInner::TcpStream(tcp_stream)
        }
    }

    #[inline]
    pub fn advance(
        self,
    ) -> (Option<Either<EstablishedConnection, Self>>, bool) { // bool = would block
        let handshake_result = match self.inner {
            HandshakeMachineInner::TcpStream(stream) => {
                self.tls_acceptor.accept(stream)
            },
            HandshakeMachineInner::TlsMidHandshake(handshake) => {
                handshake.handshake()
            },
        };

        match handshake_result {
            Ok(stream) => {
                let established = EstablishedConnection::new(
                    Stream::TlsStream(stream)
                );

                (Some(Either::Left(established)), false)
            },
            Err(native_tls::HandshakeError::WouldBlock(handshake)) => {
                let machine = Self {
                    tls_acceptor: self.tls_acceptor,
                    inner: HandshakeMachineInner::TlsMidHandshake(handshake),
                };

                (Some(Either::Right(machine)), true)
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
    pub inner: Either<EstablishedConnection, TlsHandshakeMachine>,
}


impl Connection {
    #[inline]
    pub fn new(
        opt_tls_acceptor: &Option<Arc<TlsAcceptor>>,
        valid_until: ValidUntil,
        tcp_stream: TcpStream,
    ) -> Self {
        // Setup handshake machine if TLS is requested
        let inner = if let Some(tls_acceptor) = opt_tls_acceptor {
            Either::Right(TlsHandshakeMachine::new(tls_acceptor.clone(), tcp_stream))
        } else {
            Either::Left(EstablishedConnection::new(Stream::TcpStream(tcp_stream)))
        };

        Self {
            valid_until,
            inner,
        }
    }
}


pub type ConnectionMap = HashMap<Token, Connection>;