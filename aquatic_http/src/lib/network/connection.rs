use std::net::{SocketAddr};
use std::io::ErrorKind;
use std::io::{Read, Write};

use aquatic_http_protocol::response::{FailureResponse, Response};
use hashbrown::HashMap;
use mio::{Token, Poll};
use mio::net::TcpStream;
use log::*;

use aquatic_http_protocol::request::{Request, RequestParseError};
use rustls::{ServerSession, Session, TLSError};

use crate::common::*;

pub enum ConnectionPollStatus {
    Keep,
    Remove,
}

#[derive(Debug)]
pub enum RequestReadError {
    NeedMoreData,
    StreamEnded,
    Parse(anyhow::Error),
    Io(::std::io::Error),
    Tls(TLSError),
}

pub struct Connection {
    pub valid_until: ValidUntil,
    pub peer_addr: SocketAddr,
    stream: TcpStream,
    buf: Vec<u8>,
    bytes_read: usize,
    tls_session: Option<rustls::ServerSession>,
}


impl Connection {
    #[inline]
    pub fn new(
        stream: TcpStream,
        peer_addr: SocketAddr,
        valid_until: ValidUntil,
        tls_session: Option<ServerSession>,
    ) -> Self {
        Self {
            valid_until,
            peer_addr,
            stream,
            buf: Vec::new(),
            bytes_read: 0,
            tls_session,
        }
    }

    pub fn handle_read_event(
        &mut self,
        socket_worker_index: usize,
        request_channel_sender: &RequestChannelSender,
        local_responses: &mut Vec<(ConnectionMeta, Response)>,
        poll_token: Token,
    ) -> ConnectionPollStatus {
        loop {
            if let ConnectionPollStatus::Remove = self.read_tls(){
                return ConnectionPollStatus::Remove;
            }
            match self.read_request(){
                Ok(request) => {
                    let meta = ConnectionMeta {
                        worker_index: socket_worker_index,
                        poll_token,
                        peer_addr: self.peer_addr
                    };

                    debug!("read request, sending to handler");

                    if let Err(err) = request_channel_sender
                        .send((meta, request))
                    {
                        error!(
                            "RequestChannelSender: couldn't send message: {:?}",
                            err
                        );
                    }

                    return ConnectionPollStatus::Keep;
                },
                Err(RequestReadError::NeedMoreData) => {
                    info!("need more data");

                    // Stop reading data (defer to later events)
                    return ConnectionPollStatus::Keep;
                },
                Err(RequestReadError::Parse(err)) => {
                    info!("error reading request (invalid): {:#?}", err);

                    let meta = ConnectionMeta {
                        worker_index: socket_worker_index,
                        poll_token,
                        peer_addr: self.peer_addr
                    };

                    let response = FailureResponse {
                        failure_reason: "invalid request".to_string()
                    };

                    local_responses.push(
                        (meta, Response::Failure(response))
                    );

                    return ConnectionPollStatus::Keep;
                },
                Err(RequestReadError::StreamEnded) => {
                    ::log::debug!("stream ended");

                    return ConnectionPollStatus::Remove;
                },
                Err(RequestReadError::Io(err)) => {
                    ::log::info!("error reading request (io): {}", err);
            
                    return ConnectionPollStatus::Remove;
                },
                Err(RequestReadError::Tls(err)) => {
                    ::log::info!("error reading request (tls): {}", err);
            
                    return ConnectionPollStatus::Remove;
                },
            }
        }
    }

    fn read_tls(&mut self) -> ConnectionPollStatus {
        if let Some(tls_session) = self.tls_session.as_mut() {
            match tls_session.read_tls(&mut self.stream) {
                Err(err) => {
                    if let ErrorKind::WouldBlock = err.kind() {
                        return ConnectionPollStatus::Keep;
                    }

                    error!("tls read error {:?}", err);

                    return ConnectionPollStatus::Remove;
                }
                Ok(0) => {
                    return ConnectionPollStatus::Remove;
                }
                Ok(_) => { }
            };

            if let Err(err) = tls_session.process_new_packets() {
                error!("cannot process tls packet: {:?}", err);

                // last gasp write to send any alerts
                // self.do_tls_write_and_handle_error(); // FIXME

                return ConnectionPollStatus::Remove
            }
        }

        ConnectionPollStatus::Keep
    }

    fn read_request(&mut self) -> Result<Request, RequestReadError> {
        // FIXME: dubious method
        if (self.buf.len() - self.bytes_read < 512) & (self.buf.len() <= 3072){
            self.buf.extend_from_slice(&[0; 1024]);
        }

        if let Some(tls_session) = self.tls_session.as_mut() {
            match tls_session.process_new_packets() {
                Err(err) => {
                    return Err(RequestReadError::Tls(err))
                },
                Ok(()) => {

                    match tls_session.read(&mut self.buf[self.bytes_read..]){
                        Ok(0) => {
                            // FIXME: what if all data was read?
                            return Err(RequestReadError::NeedMoreData);
                        }
                        Ok(bytes) => {
                            self.bytes_read += bytes;

                            ::log::debug!("read_request read {} bytes", bytes);
                        },
                        Err(err) if err.kind() == ErrorKind::WouldBlock => {
                            return Err(RequestReadError::NeedMoreData);
                        },
                        Err(err) => {
                            Self::clear_buffer(&mut self.buf, &mut self.bytes_read);

                            return Err(RequestReadError::Io(err));
                        }
                    } 
                },
            }
        } else {
            match self.stream.read(&mut self.buf[self.bytes_read..]){
                Ok(0) => {
                    Self::clear_buffer(&mut self.buf, &mut self.bytes_read);

                    return Err(RequestReadError::StreamEnded);
                }
                Ok(bytes) => {
                    self.bytes_read += bytes;

                    ::log::debug!("read_request read {} bytes", bytes);
                },
                Err(err) if err.kind() == ErrorKind::WouldBlock => {
                    return Err(RequestReadError::NeedMoreData);
                },
                Err(err) => {
                    Self::clear_buffer(&mut self.buf, &mut self.bytes_read);

                    return Err(RequestReadError::Io(err));
                }
            }
        }

        match Request::from_bytes(&self.buf[..self.bytes_read]){
            Ok(request) => {
                Self::clear_buffer(&mut self.buf, &mut self.bytes_read);

                Ok(request)
            },
            Err(RequestParseError::NeedMoreData) => {
                Err(RequestReadError::NeedMoreData)
            },
            Err(RequestParseError::Invalid(err)) => {
                Self::clear_buffer(&mut self.buf, &mut self.bytes_read);

                Err(RequestReadError::Parse(err))
            },
        }
    }

    pub fn send_response(&mut self, body: &[u8]) -> ::std::io::Result<()> {
        if let Some(tls_session) = self.tls_session.as_mut() {
            Self::send_response_inner(tls_session, body)
        } else {
            Self::send_response_inner(&mut self.stream, body)
        }
    }

    fn send_response_inner(stream: &mut impl Write, body: &[u8]) -> ::std::io::Result<()> {
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

        let bytes_written = stream.write(&response)?;

        if bytes_written != response.len() {
            ::log::error!(
                "send_response: only {} out of {} bytes written",
                bytes_written,
                response.len()
            );
        }

        stream.flush()?;

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
    pub fn clear_buffer(buf: &mut Vec<u8>, bytes_read: &mut usize){
        *bytes_read = 0;
        *buf = Vec::new();
    }

    pub fn deregister(&mut self, poll: &mut Poll) -> ::std::io::Result<()> {
        poll.registry().deregister(&mut self.stream)
    }
}


pub type ConnectionMap = HashMap<Token, Connection>;


#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_num_digits_in_usize(){
        let f = Connection::num_digits_in_usize;

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