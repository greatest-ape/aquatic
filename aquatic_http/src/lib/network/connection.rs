use std::net::{SocketAddr};
use std::io::ErrorKind;
use std::io::{Read, Write};
use std::sync::Arc;

use hashbrown::HashMap;
use mio::Token;
use mio::net::TcpStream;
use native_tls::{TlsAcceptor, MidHandshakeTlsStream};

use aquatic_common_tcp::network::stream::Stream;

use crate::common::*;
use crate::protocol::request::Request;


#[derive(Debug)]
pub enum RequestReadError {
    NeedMoreData,
    Invalid(anyhow::Error),
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
    #[inline]
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

                ::log::debug!("read_request read {} bytes", bytes_read);
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
                if let Some(path) = http_request.path {
                    let res_request = Request::from_http_get_path(path);

                    self.clear_buffer();

                    res_request.map_err(RequestReadError::Invalid)
                } else {
                    self.clear_buffer();

                    Err(RequestReadError::Invalid(anyhow::anyhow!("no http path")))
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

    #[inline]
    pub fn clear_buffer(&mut self){
        self.bytes_read = 0;
        self.buf = Vec::new();
    }
}


pub enum TlsHandshakeMachineError {
    WouldBlock(TlsHandshakeMachine),
    Failure(native_tls::Error)
}


enum TlsHandshakeMachineInner {
    TcpStream(TcpStream),
    TlsMidHandshake(MidHandshakeTlsStream<TcpStream>),
}


pub struct TlsHandshakeMachine {
    tls_acceptor: Arc<TlsAcceptor>,
    inner: TlsHandshakeMachineInner,
}


impl <'a>TlsHandshakeMachine {
    #[inline]
    fn new(
        tls_acceptor: Arc<TlsAcceptor>,
        tcp_stream: TcpStream
    ) -> Self {
        Self {
            tls_acceptor,
            inner: TlsHandshakeMachineInner::TcpStream(tcp_stream)
        }
    }

    /// Attempt to establish a TLS connection. On a WouldBlock error, return
    /// the machine wrapped in an error for later attempts.
    pub fn establish_tls(self) -> Result<EstablishedConnection, TlsHandshakeMachineError> {
        let handshake_result = match self.inner {
            TlsHandshakeMachineInner::TcpStream(stream) => {
                self.tls_acceptor.accept(stream)
            },
            TlsHandshakeMachineInner::TlsMidHandshake(handshake) => {
                handshake.handshake()
            },
        };

        match handshake_result {
            Ok(stream) => {
                let established = EstablishedConnection::new(
                    Stream::TlsStream(stream)
                );

                Ok(established)
            },
            Err(native_tls::HandshakeError::WouldBlock(handshake)) => {
                let inner = TlsHandshakeMachineInner::TlsMidHandshake(
                    handshake
                );
                
                let machine = Self {
                    tls_acceptor: self.tls_acceptor,
                    inner,
                };

                Err(TlsHandshakeMachineError::WouldBlock(machine))
            },
            Err(native_tls::HandshakeError::Failure(err)) => {
                Err(TlsHandshakeMachineError::Failure(err))
            }
        }
    }
}


enum ConnectionInner {
    Established(EstablishedConnection),
    InProgress(TlsHandshakeMachine),
}


pub struct Connection {
    pub valid_until: ValidUntil,
    inner: ConnectionInner,
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
            ConnectionInner::InProgress(
                TlsHandshakeMachine::new(tls_acceptor.clone(), tcp_stream)
            )
        } else {
            ConnectionInner::Established(
                EstablishedConnection::new(Stream::TcpStream(tcp_stream))
            )
        };

        Self {
            valid_until,
            inner,
        }
    }

    #[inline]
    pub fn from_established(
        valid_until: ValidUntil,
        established: EstablishedConnection,
    ) -> Self {
        Self {
            valid_until,
            inner: ConnectionInner::Established(established)
        }
    }

    #[inline]
    pub fn from_in_progress(
        valid_until: ValidUntil,
        machine: TlsHandshakeMachine,
    ) -> Self {
        Self {
            valid_until,
            inner: ConnectionInner::InProgress(machine)
        }
    }

    #[inline]
    pub fn get_established(&mut self) -> Option<&mut EstablishedConnection> {
        if let ConnectionInner::Established(ref mut established) = self.inner {
            Some(established)
        } else {
            None
        }
    }

    #[inline]
    pub fn get_in_progress(self) -> Option<TlsHandshakeMachine> {
        if let ConnectionInner::InProgress(machine) = self.inner {
            Some(machine)
        } else {
            None
        }
    }
}


pub type ConnectionMap = HashMap<Token, Connection>;