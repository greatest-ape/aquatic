use std::net::{SocketAddr};
use std::io::ErrorKind;
use std::io::{Read, Write};
use std::sync::Arc;

use hashbrown::HashMap;
use mio::Token;
use mio::net::TcpStream;
use native_tls::{TlsAcceptor, MidHandshakeTlsStream};

use aquatic_http_protocol::request::{Request, RequestParseError};

use crate::common::*;

use super::stream::Stream;


#[derive(Debug)]
pub enum RequestReadError {
    NeedMoreData,
    StreamEnded,
    Parse(anyhow::Error),
    Io(::std::io::Error),
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
        if (self.buf.len() - self.bytes_read < 512) & (self.buf.len() <= 3072){
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

        match Request::from_bytes(&self.buf[..self.bytes_read]){
            Ok(request) => {
                self.clear_buffer();

                Ok(request)
            },
            Err(RequestParseError::NeedMoreData) => {
                Err(RequestReadError::NeedMoreData)
            },
            Err(RequestParseError::Invalid(err)) => {
                self.clear_buffer();

                Err(RequestReadError::Parse(err))
            },
        }
    }

    pub fn send_response(&mut self, body: &[u8]) -> ::std::io::Result<()> {
        let content_len = body.len() + 2; // 2 is for newlines at end
        let content_len_num_digits = Self::num_digits_in_usize(content_len);

        let mut response = Vec::with_capacity(
            39 + content_len_num_digits + body.len()
        );

        response.extend_from_slice(b"HTTP/1.1 200 OK\r\nContent-Length: ");
        ::itoa::write(&mut response, content_len)?;
        response.extend_from_slice(b"\r\n\r\n");
        response.extend_from_slice(body);
        response.extend_from_slice(b"\r\n");

        let bytes_written = self.stream.write(&response)?;

        if bytes_written != response.len() {
            ::log::error!(
                "send_response: only {} out of {} bytes written",
                bytes_written,
                response.len()
            );
        }

        self.stream.flush()?;

        Ok(())
    }

    fn num_digits_in_usize(mut number: usize) -> usize {
        let mut num_digits = 1usize;

        while number >= 10 {
            num_digits += 1;

            number /= 10;
        }

        num_digits
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

                ::log::debug!("established tcp connection");

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
            ::log::debug!("established tcp connection");

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

    /// Takes ownership since TlsStream needs ownership of TcpStream
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


#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_num_digits_in_usize(){
        let f = EstablishedConnection::num_digits_in_usize;

        assert_eq!(f(0), 1);
        assert_eq!(f(1), 1);
        assert_eq!(f(9), 1);
        assert_eq!(f(10), 2);
        assert_eq!(f(11), 2);
        assert_eq!(f(99), 2);
        assert_eq!(f(100), 3);
        assert_eq!(f(101), 3);
        assert_eq!(f(1000), 4);
    }
}