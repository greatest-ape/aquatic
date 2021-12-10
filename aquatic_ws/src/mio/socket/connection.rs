use std::{sync::Arc, io::ErrorKind, net::Shutdown, marker::PhantomData};

use aquatic_common::ValidUntil;
use aquatic_ws_protocol::{InMessage, OutMessage};
use mio::{net::TcpStream, Poll, Token, Interest};
use rustls::{ServerConfig, ServerConnection};
use tungstenite::{HandshakeError, handshake::{MidHandshake, server::NoCallback}, ServerHandshake, protocol::WebSocketConfig};

use crate::common::ConnectionMeta;

type TlsStream = rustls::StreamOwned<ServerConnection, TcpStream>;

pub type ConnectionReadResult<T> = ::std::io::Result<ConnectionReadStatus<T>>;

pub trait RegistryStatus {}

pub struct Registered;

impl RegistryStatus for Registered {}

pub struct NotRegistered;

impl RegistryStatus for NotRegistered {}

pub enum ConnectionReadStatus<T> {
    Ok(T),
    WouldBlock(T),
}

type HandshakeResult<S> = Result<tungstenite::WebSocket<S>, HandshakeError<ServerHandshake<S, NoCallback>>>;

struct TlsHandshaking<R: RegistryStatus> {
    tls_conn: ServerConnection,
    ws_config: WebSocketConfig,
    tcp_stream: TcpStream,
    phantom_data: PhantomData<R>,
}

impl TlsHandshaking<NotRegistered> {
    fn new(
        tls_config: Arc<ServerConfig>,
        ws_config: WebSocketConfig,
        stream: TcpStream,
    ) -> Self {
        Self {
            tls_conn: ServerConnection::new(tls_config).unwrap(),
            ws_config,
            tcp_stream: stream,
            phantom_data: PhantomData::default(),
        }
    }

    fn read(mut self) -> ConnectionReadResult<ConnectionState<NotRegistered>> {
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

    fn register(mut self, poll: &mut Poll, token: Token) -> TlsHandshaking<Registered> {
        poll.registry().register(&mut self.tcp_stream, token, Interest::READABLE).unwrap();

        TlsHandshaking {
            tls_conn: self.tls_conn,
            ws_config: self.ws_config,
            tcp_stream: self.tcp_stream,
            phantom_data: PhantomData::default(),
        }
    }

    fn close(self) {
        let _ = self.tcp_stream.shutdown(Shutdown::Both);
    }
}

impl TlsHandshaking<Registered> {
    fn deregister(mut self, poll: &mut Poll) -> TlsHandshaking<NotRegistered> {
        poll.registry().deregister(&mut self.tcp_stream).unwrap();

        TlsHandshaking {
            tls_conn: self.tls_conn,
            ws_config: self.ws_config,
            tcp_stream: self.tcp_stream,
            phantom_data: PhantomData::default(),
        }
    }
}

struct WsHandshaking<R: RegistryStatus> {
    mid_handshake: MidHandshake<ServerHandshake<TlsStream, NoCallback>>,
    phantom_data: PhantomData<R>,
}

impl WsHandshaking<NotRegistered> {
    fn read(self) -> ConnectionReadResult<ConnectionState<NotRegistered>> {
        Self::handle_handshake_result(self.mid_handshake.handshake())
    }

    fn handle_handshake_result(handshake_result: HandshakeResult<TlsStream>) -> ConnectionReadResult<ConnectionState<NotRegistered>> {
        match handshake_result {
            Ok(web_socket) => {
                let conn = ConnectionState::WsConnection(WsConnection {
                    web_socket,
                    phantom_data: PhantomData::default(),
                });

                Ok(ConnectionReadStatus::Ok(conn))
            },
            Err(HandshakeError::Interrupted(mid_handshake)) => {
                let conn = ConnectionState::WsHandshaking(WsHandshaking {
                    mid_handshake,
                    phantom_data: PhantomData::default(),
                });

                Ok(ConnectionReadStatus::WouldBlock(conn))
            }
            Err(HandshakeError::Failure(err)) => {
                return Err(std::io::Error::new(ErrorKind::InvalidData, err))
            }
        }
    }

    fn register(mut self, poll: &mut Poll, token: Token) -> WsHandshaking<Registered> {
        let tcp_stream = &mut self.mid_handshake.get_mut().get_mut().sock;

        poll.registry().register(tcp_stream, token, Interest::READABLE).unwrap();

        WsHandshaking {
            mid_handshake: self.mid_handshake,
            phantom_data: PhantomData::default(),
        }
    }

    fn close(mut self) {
        let tcp_stream = &mut self.mid_handshake.get_mut().get_mut().sock;
        
        let _ = tcp_stream.shutdown(Shutdown::Both);
    }
}

impl WsHandshaking<Registered> {
    fn deregister(mut self, poll: &mut Poll) -> WsHandshaking<NotRegistered> {
        let tcp_stream = &mut self.mid_handshake.get_mut().get_mut().sock;

        poll.registry().deregister(tcp_stream).unwrap();

        WsHandshaking {
            mid_handshake: self.mid_handshake,
            phantom_data: PhantomData::default(),
        }
    }
}

struct WsConnection<R: RegistryStatus> {
    web_socket: tungstenite::WebSocket<TlsStream>,
    phantom_data: PhantomData<R>,
}

impl WsConnection<NotRegistered> {
    fn read<F>(mut self, message_handler: &mut F, meta: ConnectionMeta) -> ConnectionReadResult<ConnectionState<NotRegistered>> where F: FnMut(ConnectionMeta, InMessage) {
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

    fn register(mut self, poll: &mut Poll, token: Token) -> WsConnection<Registered> {
        poll.registry().register(self.web_socket.get_mut().get_mut(), token, Interest::READABLE).unwrap();

        WsConnection {
            web_socket: self.web_socket,
            phantom_data: PhantomData::default(),
        }
    }

    fn close(mut self) {
        let _ = self.web_socket.close(None);

        loop {
            if let Err(_) = self.web_socket.write_pending() {
                break;
            }
        }
    }
}

impl WsConnection<Registered> {
    fn deregister(mut self, poll: &mut Poll) -> WsConnection<NotRegistered> {
        poll.registry().deregister(self.web_socket.get_mut().get_mut()).unwrap();

        WsConnection {
            web_socket: self.web_socket,
            phantom_data: PhantomData::default(),
        }
    }
}

enum ConnectionState<R: RegistryStatus> {
    TlsHandshaking(TlsHandshaking<R>),
    WsHandshaking(WsHandshaking<R>),
    WsConnection(WsConnection<R>)
}


pub struct Connection<R: RegistryStatus> {
    pub valid_until: ValidUntil,
    pub meta: ConnectionMeta,
    state: ConnectionState<R>,
    phantom_data: PhantomData<R>,
}

impl Connection<NotRegistered> {
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
            phantom_data: PhantomData::default(),
        }
    }

    pub fn read<F>(mut self, message_handler: &mut F) -> ConnectionReadResult<Connection<NotRegistered>> where F: FnMut(ConnectionMeta, InMessage) {
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

    pub fn register(self, poll: &mut Poll, token: Token) -> Connection<Registered> {
        let state = match self.state {
            ConnectionState::TlsHandshaking(inner) => ConnectionState::TlsHandshaking(inner.register(poll, token)),
            ConnectionState::WsHandshaking(inner) => ConnectionState::WsHandshaking(inner.register(poll, token)),
            ConnectionState::WsConnection(inner) => ConnectionState::WsConnection(inner.register(poll, token)),
        };

        Connection {
            valid_until: self.valid_until,
            meta: self.meta,
            state,
            phantom_data: PhantomData::default(),
        }
    }

    pub fn close(self) {
        match self.state {
            ConnectionState::TlsHandshaking(inner) => inner.close(),
            ConnectionState::WsHandshaking(inner) => inner.close(),
            ConnectionState::WsConnection(inner) => inner.close(),
        }
    }
}

impl Connection<Registered> {
    pub fn write(&mut self, message: OutMessage) -> ::std::io::Result<()> {
        if let ConnectionState::WsConnection(WsConnection { ref mut web_socket , ..}) = self.state {
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

    pub fn deregister(self, poll: &mut Poll) -> Connection<NotRegistered> {
        let state = match self.state {
            ConnectionState::TlsHandshaking(inner) => ConnectionState::TlsHandshaking(inner.deregister(poll)),
            ConnectionState::WsHandshaking(inner) => ConnectionState::WsHandshaking(inner.deregister(poll)),
            ConnectionState::WsConnection(inner) => ConnectionState::WsConnection(inner.deregister(poll)),
        };

        Connection {
            valid_until: self.valid_until,
            meta: self.meta,
            state,
            phantom_data: PhantomData::default(),
        }
    }
}