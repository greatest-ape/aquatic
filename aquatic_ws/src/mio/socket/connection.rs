use std::{sync::Arc, io::ErrorKind, net::Shutdown};

use aquatic_common::ValidUntil;
use aquatic_ws_protocol::{InMessage, OutMessage};
use mio::{net::TcpStream, Poll, Token, Interest};
use rustls::{ServerConfig, ServerConnection};
use tungstenite::{HandshakeError, handshake::{MidHandshake, server::NoCallback}, ServerHandshake, protocol::WebSocketConfig};

use crate::common::ConnectionMeta;

type TlsStream = rustls::StreamOwned<ServerConnection, TcpStream>;

pub type ConnectionReadResult<T> = ::std::io::Result<ConnectionReadStatus<T>>;

pub enum ConnectionReadStatus<T> {
    Ok(T),
    WouldBlock(T),
}

type HandshakeResult<S> = Result<tungstenite::WebSocket<S>, HandshakeError<ServerHandshake<S, NoCallback>>>;

struct TlsHandshaking {
    tls_conn: ServerConnection,
    ws_config: WebSocketConfig,
    tcp_stream: TcpStream,
}

impl TlsHandshaking {
    fn new(
        tls_config: Arc<ServerConfig>,
        ws_config: WebSocketConfig,
        stream: TcpStream,
    ) -> Self {
        Self {
            tls_conn: ServerConnection::new(tls_config).unwrap(),
            ws_config,
            tcp_stream: stream,
        }
    }

    fn read(mut self) -> ConnectionReadResult<ConnectionState> {
        match self.tls_conn.read_tls(&mut self.tcp_stream) {
            Ok(0) => {
                return Err(::std::io::Error::new(ErrorKind::ConnectionReset, "Connection closed"))
            }
            Ok(_) => {
                match self.tls_conn.process_new_packets() {
                    Ok(_) => {
                        while self.tls_conn.wants_write() {
                            self.tls_conn.write_tls(&mut self.tcp_stream)?;
                        }

                        if self.tls_conn.is_handshaking() {
                            Ok(ConnectionReadStatus::WouldBlock(ConnectionState::TlsHandshaking(self)))
                        } else {
                            let tls_stream = TlsStream::new(self.tls_conn, self.tcp_stream);

                            WsHandshaking::handle_handshake_result(tungstenite::accept_with_config(tls_stream, Some(self.ws_config)))
                        }
                    }
                    Err(err) => {
                        let _ = self.tls_conn.write_tls(&mut self.tcp_stream);

                        Err(::std::io::Error::new(ErrorKind::InvalidData, err))
                    }
                }
            }
            Err(err) if err.kind() == ErrorKind::WouldBlock => {
                return Ok(ConnectionReadStatus::WouldBlock(ConnectionState::TlsHandshaking(self)))
            }
            Err(err) => return Err(err),
        }
    }

    fn close(&mut self, poll: &mut Poll) {
        let _ = self.tcp_stream.shutdown(Shutdown::Both);

        self.deregister(poll);
    }

    fn register(&mut self, poll: &mut Poll, token: Token) {
        poll.registry().register(&mut self.tcp_stream, token, Interest::READABLE).unwrap();
    }

    fn deregister(&mut self, poll: &mut Poll) {
        poll.registry().deregister(&mut self.tcp_stream).unwrap();
    }
}

struct WsHandshaking {
    mid_handshake: MidHandshake<ServerHandshake<TlsStream, NoCallback>>,
}

impl WsHandshaking {
    fn read(self) -> ConnectionReadResult<ConnectionState> {
        Self::handle_handshake_result(self.mid_handshake.handshake())
    }

    fn handle_handshake_result(handshake_result: HandshakeResult<TlsStream>) -> ConnectionReadResult<ConnectionState> {
        match handshake_result {
            Ok(web_socket) => {
                let conn = ConnectionState::WsConnection(WsConnection {
                    web_socket
                });

                Ok(ConnectionReadStatus::Ok(conn))
            },
            Err(HandshakeError::Interrupted(mid_handshake)) => {
                let conn = ConnectionState::WsHandshaking(WsHandshaking {
                    mid_handshake,
                });

                Ok(ConnectionReadStatus::WouldBlock(conn))
            }
            Err(HandshakeError::Failure(err)) => {
                return Err(std::io::Error::new(ErrorKind::InvalidData, err))
            }
        }
    }

    fn close(&mut self, poll: &mut Poll) {
        let tcp_stream = &mut self.mid_handshake.get_mut().get_mut().sock;
        
        let _ = tcp_stream.shutdown(Shutdown::Both);

        self.deregister(poll);
    }

    fn register(&mut self, poll: &mut Poll, token: Token) {
        let tcp_stream = &mut self.mid_handshake.get_mut().get_mut().sock;

        poll.registry().register(tcp_stream, token, Interest::READABLE).unwrap();
    }

    fn deregister(&mut self, poll: &mut Poll) {
        let tcp_stream = &mut self.mid_handshake.get_mut().get_mut().sock;

        poll.registry().deregister(tcp_stream).unwrap();
    }
}

struct WsConnection {
    web_socket: tungstenite::WebSocket<TlsStream>,
}

impl WsConnection {
    fn read<F>(mut self, message_handler: &mut F, meta: ConnectionMeta) -> ConnectionReadResult<ConnectionState> where F: FnMut(ConnectionMeta, InMessage) {
        match self.web_socket.read_message() {
            Ok(message) => {
                match InMessage::from_ws_message(message) {
                    Ok(message) => {
                        message_handler(meta, message);

                        Ok(ConnectionReadStatus::Ok(ConnectionState::WsConnection(self)))
                    }
                    Err(err) => {
                        Err(std::io::Error::new(ErrorKind::InvalidData, err))
                    }
                }
            },
            Err(tungstenite::Error::Io(err)) if err.kind() == ErrorKind::WouldBlock => {
                let conn = ConnectionState::WsConnection(self);

                Ok(ConnectionReadStatus::WouldBlock(conn))
            }
            Err(tungstenite::Error::Io(err)) => {
                Err(err)
            }
            Err(err) => {
                Err(std::io::Error::new(ErrorKind::InvalidData, err))
            }
        }
    }

    fn close(&mut self, poll: &mut Poll) {
        let _ = self.web_socket.close(None);

        loop {
            if let Err(_) = self.web_socket.write_pending() {
                break;
            }
        }

        self.deregister(poll)
    }

    fn register(&mut self, poll: &mut Poll, token: Token) {
        poll.registry().register(self.web_socket.get_mut().get_mut(), token, Interest::READABLE).unwrap();
    }

    fn deregister(&mut self, poll: &mut Poll) {
        poll.registry().deregister(self.web_socket.get_mut().get_mut()).unwrap();
    }
}

enum ConnectionState {
    TlsHandshaking(TlsHandshaking),
    WsHandshaking(WsHandshaking),
    WsConnection(WsConnection)
}

pub struct Connection {
    pub valid_until: ValidUntil,
    pub meta: ConnectionMeta,
    state: ConnectionState,
}

impl Connection {
    pub fn new(
        tls_config: Arc<ServerConfig>,
        ws_config: WebSocketConfig,
        tcp_stream: TcpStream,
        valid_until: ValidUntil,
        meta: ConnectionMeta,
    )  -> Self {
        let state = ConnectionState::TlsHandshaking(TlsHandshaking::new(tls_config, ws_config, tcp_stream));

        Self {
            valid_until,
            meta,
            state,
        }
    }

    pub fn read<F>(mut self, message_handler: &mut F) -> ConnectionReadResult<Connection> where F: FnMut(ConnectionMeta, InMessage) {
        loop {
            let result = match self.state {
                ConnectionState::TlsHandshaking(inner) => inner.read(),
                ConnectionState::WsHandshaking(inner) => inner.read(),
                ConnectionState::WsConnection(inner) => inner.read(message_handler, self.meta),
            };

            match result {
                Ok(ConnectionReadStatus::Ok(state)) => {
                    self.state = state;
                }
                Ok(ConnectionReadStatus::WouldBlock(state)) => {
                    self.state = state;

                    return Ok(ConnectionReadStatus::WouldBlock(self));
                }
                Err(err) => {
                    return Err(err);
                }
            }
        }
    }

    pub fn write(&mut self, message: OutMessage) -> ::std::io::Result<()> {
        if let ConnectionState::WsConnection(WsConnection { ref mut web_socket }) = self.state {
            match web_socket.write_message(message.to_ws_message()) {
                Ok(_) => Ok(()),
                Err(tungstenite::Error::Io(err)) => {
                    Err(err)
                }
                Err(err) => {
                    Err(std::io::Error::new(ErrorKind::Other, err))
                }
            }
        } else {
            Err(std::io::Error::new(ErrorKind::NotConnected, "WebSocket connection not established"))
        }
    }

    pub fn close(&mut self, poll: &mut Poll) {
        match self.state {
            ConnectionState::TlsHandshaking(ref mut inner) => inner.close(poll),
            ConnectionState::WsHandshaking(ref mut inner) => inner.close(poll),
            ConnectionState::WsConnection(ref mut inner) => inner.close(poll),
        }
    }

    pub fn register(&mut self, poll: &mut Poll, token: Token) {
        match self.state {
            ConnectionState::TlsHandshaking(ref mut inner) => inner.register(poll, token),
            ConnectionState::WsHandshaking(ref mut inner) => inner.register(poll, token),
            ConnectionState::WsConnection(ref mut inner) => inner.register(poll, token),
        }
    }

    pub fn deregister(&mut self, poll: &mut Poll) {
        match self.state {
            ConnectionState::TlsHandshaking(ref mut inner) => inner.deregister(poll),
            ConnectionState::WsHandshaking(ref mut inner) => inner.deregister(poll),
            ConnectionState::WsConnection(ref mut inner) => inner.deregister(poll),
        }
    }
}
