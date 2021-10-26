use std::io::{Read, Write};
use std::net::SocketAddr;

use mio::net::TcpStream;
use native_tls::TlsStream;

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
